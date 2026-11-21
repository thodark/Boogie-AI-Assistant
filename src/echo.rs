use std::collections::VecDeque;

const HISTORY_SECS: f64 = 1.0;
const ECHO_RATIO: f32 = 1.4;

pub struct EchoGuard {
    output_history: VecDeque<(f32, f64)>,
    learned_ratio: f32,
    sample_count: u32,
}

impl EchoGuard {
    pub fn new() -> Self {
        Self {
            output_history: VecDeque::new(),
            learned_ratio: ECHO_RATIO,
            sample_count: 0,
        }
    }

    pub fn note_output(&mut self, _samples: &[i16], sample_rate: u32, level: f32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.output_history.push_back((level, now));
        let cutoff = now - HISTORY_SECS;
        while self
            .output_history
            .front()
            .is_some_and(|(_, t)| *t < cutoff)
        {
            self.output_history.pop_front();
        }
        let _ = sample_rate;
    }

    pub fn is_user_speech(&mut self, _samples: &[i16], _sample_rate: u32, mic_level: f32) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let cutoff = now - HISTORY_SECS;
        while self
            .output_history
            .front()
            .is_some_and(|(_, t)| *t < cutoff)
        {
            self.output_history.pop_front();
        }

        let recent_output = self
            .output_history
            .iter()
            .map(|(l, _)| *l)
            .fold(0.0f32, f32::max);

        if recent_output < 0.01 {
            return mic_level > 0.02;
        }

        let ratio = mic_level / (recent_output + 1e-6);
        if self.sample_count < 100 {
            self.sample_count += 1;
            if ratio > self.learned_ratio * 0.8 {
                self.learned_ratio = self.learned_ratio * 0.95 + ratio * 0.05;
            }
        }

        ratio > self.learned_ratio
    }

    pub fn reset(&mut self) {
        self.output_history.clear();
    }
}

impl Default for EchoGuard {
    fn default() -> Self {
        Self::new()
    }
}
