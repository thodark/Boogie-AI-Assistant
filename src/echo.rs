use std::time::Instant;

const SPEECH_THRESHOLD: f32 = 0.018;
const BARGE_IN_RATIO: f32 = 1.45;
const SPEAKER_TAIL_SECS: f64 = 0.40;
const SPEECH_HANGOVER_SECS: f64 = 0.45;
const STREAM_END_SILENCE_SECS: f64 = 0.75;
const PTT_RELEASE_TAIL_SECS: f64 = 0.12;

pub struct EchoGuard {
    recent_output_level: f32,
    last_speaker_active: Option<Instant>,
    last_user_voice: Option<Instant>,
    stream_active: bool,
    learned_ratio: f32,
    sample_count: u32,
}

impl EchoGuard {
    pub fn new() -> Self {
        Self {
            recent_output_level: 0.0,
            last_speaker_active: None,
            last_user_voice: None,
            stream_active: false,
            learned_ratio: BARGE_IN_RATIO,
            sample_count: 0,
        }
    }

    /// Record output audio level when speaker chunks arrive.
    pub fn note_output(&mut self, level: f32) {
        self.recent_output_level = self.recent_output_level.max(level);
        self.last_speaker_active = Some(Instant::now());
    }

    /// Evaluates whether a mic frame should be transmitted to Gemini Live.
    /// Uses real speaker buffer occupancy (`speaker_playing`) plus an acoustic room tail
    /// hangover, and keeps mic open for 400ms after user speech ends so final syllables aren't clipped.
    pub fn evaluate_mic(
        &mut self,
        mic_level: f32,
        speaker_playing: bool,
        now: Instant,
    ) -> MicDecision {
        if speaker_playing {
            self.last_speaker_active = Some(now);
        }

        let in_speaker_tail = speaker_playing
            || self
                .last_speaker_active
                .is_some_and(|t| now.duration_since(t).as_secs_f64() < SPEAKER_TAIL_SECS);

        if !in_speaker_tail {
            // Decay remembered speaker level smoothly once silent
            self.recent_output_level *= 0.85;
            if self.recent_output_level < 0.005 {
                self.recent_output_level = 0.0;
            }
        }

        let voice_detected = if !in_speaker_tail || self.recent_output_level < 0.01 {
            mic_level > SPEECH_THRESHOLD
        } else {
            let ratio = mic_level / (self.recent_output_level + 1e-6);
            if self.sample_count < 100 && ratio > self.learned_ratio * 0.8 && ratio < 4.0 {
                self.sample_count += 1;
                self.learned_ratio = self.learned_ratio * 0.95 + ratio * 0.05;
            }
            mic_level > 0.06 && ratio > self.learned_ratio
        };

        if voice_detected {
            self.last_user_voice = Some(now);
            self.stream_active = true;
            return MicDecision::SendAudio;
        }

        // Speech hangover: continue sending audio briefly after voice stops
        if let Some(last_voice) = self.last_user_voice {
            let silent_secs = now.duration_since(last_voice).as_secs_f64();
            if silent_secs < SPEECH_HANGOVER_SECS {
                return MicDecision::SendAudio;
            }
            if self.stream_active && silent_secs >= STREAM_END_SILENCE_SECS {
                self.stream_active = false;
                return MicDecision::SendStreamEnd;
            }
        }

        MicDecision::Suppress
    }

    pub fn evaluate_ptt(&mut self, ptt_active: bool, now: Instant) -> MicDecision {
        if ptt_active {
            self.last_user_voice = Some(now);
            self.stream_active = true;
            return MicDecision::SendAudio;
        }
        if self.stream_active {
            let elapsed = self
                .last_user_voice
                .map(|t| now.duration_since(t).as_secs_f64())
                .unwrap_or(PTT_RELEASE_TAIL_SECS);
            if elapsed < PTT_RELEASE_TAIL_SECS {
                return MicDecision::SendAudio;
            }
            self.stream_active = false;
            return MicDecision::SendStreamEnd;
        }
        MicDecision::Suppress
    }

    /// Clears pending user-speech hangover once the model begins responding,
    /// preventing speaker echo leakage or a spurious late `audioStreamEnd` mid-reply.
    pub fn on_model_response_started(&mut self) {
        self.stream_active = false;
        self.last_user_voice = None;
    }

    pub fn reset(&mut self) {
        self.recent_output_level = 0.0;
        self.last_speaker_active = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicDecision {
    SendAudio,
    SendStreamEnd,
    Suppress,
}

impl Default for EchoGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn suppresses_echo_while_speaker_is_actively_playing() {
        let mut eg = EchoGuard::new();
        let t0 = Instant::now();
        eg.note_output(0.4);
        // Even 2 seconds after chunk arrival, if speaker buffer is still playing, suppress low echo
        let t1 = t0 + Duration::from_secs(2);
        assert_eq!(eg.evaluate_mic(0.25, true, t1), MicDecision::Suppress);
        // Loud user barge-in passes through
        assert_eq!(eg.evaluate_mic(0.85, true, t1), MicDecision::SendAudio);
    }

    #[test]
    fn speech_hangover_and_stream_end_fire_in_order() {
        let mut eg = EchoGuard::new();
        let t0 = Instant::now();
        assert_eq!(eg.evaluate_mic(0.2, false, t0), MicDecision::SendAudio);
        // 200ms silence: hangover keeps audio flowing
        assert_eq!(
            eg.evaluate_mic(0.0, false, t0 + Duration::from_millis(200)),
            MicDecision::SendAudio
        );
        // 550ms silence: suppressed, not yet streamEnd
        assert_eq!(
            eg.evaluate_mic(0.0, false, t0 + Duration::from_millis(550)),
            MicDecision::Suppress
        );
        // 800ms silence: emits SendStreamEnd once
        assert_eq!(
            eg.evaluate_mic(0.0, false, t0 + Duration::from_millis(800)),
            MicDecision::SendStreamEnd
        );
        // Subsequent silence does not spam SendStreamEnd
        assert_eq!(
            eg.evaluate_mic(0.0, false, t0 + Duration::from_millis(1100)),
            MicDecision::Suppress
        );
    }

    #[test]
    fn ptt_sends_audio_while_held_and_stream_end_on_release() {
        let mut eg = EchoGuard::new();
        let t0 = Instant::now();
        assert_eq!(eg.evaluate_ptt(false, t0), MicDecision::Suppress);
        assert_eq!(eg.evaluate_ptt(true, t0), MicDecision::SendAudio);
        // Short release tail preserves trailing syllable
        assert_eq!(
            eg.evaluate_ptt(false, t0 + Duration::from_millis(50)),
            MicDecision::SendAudio
        );
        // After tail expires, emit SendStreamEnd once
        assert_eq!(
            eg.evaluate_ptt(false, t0 + Duration::from_millis(150)),
            MicDecision::SendStreamEnd
        );
        assert_eq!(
            eg.evaluate_ptt(false, t0 + Duration::from_millis(250)),
            MicDecision::Suppress
        );
    }

    #[test]
    fn model_response_started_clears_stale_hangover_and_stream_end() {
        let mut eg = EchoGuard::new();
        let t0 = Instant::now();
        assert_eq!(eg.evaluate_mic(0.2, false, t0), MicDecision::SendAudio);
        eg.on_model_response_started();
        assert_eq!(
            eg.evaluate_mic(0.0, true, t0 + Duration::from_millis(200)),
            MicDecision::Suppress
        );
        assert_eq!(
            eg.evaluate_mic(0.0, false, t0 + Duration::from_millis(800)),
            MicDecision::Suppress
        );
    }
}
