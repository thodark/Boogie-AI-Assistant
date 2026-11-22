use chrono::Timelike;
use std::time::Instant;

pub struct ProactiveEngine {
    min_silence_secs: f64,
    check_cooldown: f64,
    last_triggered: Instant,
    rotation: usize,
    enabled: bool,
}

impl ProactiveEngine {
    pub fn new(enabled: bool) -> Self {
        Self {
            min_silence_secs: 900.0,
            check_cooldown: 1200.0,
            last_triggered: Instant::now(),
            rotation: 0,
            enabled,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn should_trigger(&self, last_user_speech: Instant) -> bool {
        if !self.enabled {
            return false;
        }
        let now = Instant::now();
        let silence = now.duration_since(last_user_speech).as_secs_f64();
        let cooldown = now.duration_since(self.last_triggered).as_secs_f64();
        silence >= self.min_silence_secs && cooldown >= self.check_cooldown
    }

    pub fn mark_triggered(&mut self) {
        self.last_triggered = Instant::now();
        self.rotation += 1;
    }

    pub fn build_prompt(&self, memory: &crate::config::Memory, recent_turns: &[String]) -> String {
        let now = chrono::Local::now();
        let time_str = now.format("%A, %B %d, %Y — %I:%M %p").to_string();
        let hour = now.hour();

        let period = match hour {
            6..=11 => "morning",
            12..=17 => "afternoon",
            18..=22 => "evening",
            _ => "late night",
        };

        let mem_str = crate::config::format_memory_for_prompt(memory);
        let mem_display = if mem_str.is_empty() {
            "(no stored user data)".to_string()
        } else {
            mem_str
        };

        let focus = match self.rotation % 3 {
            0 => {
                "Focus on the user's active projects or goals if any are stored. \
                  Ask how something is going, or offer a relevant tip."
            }
            1 => {
                "Focus on the time of day and the user's wellbeing. \
                  A warm check-in, a reminder to take a break, or something timely."
            }
            _ => {
                "Focus on something genuinely interesting or useful — \
                  a fact, a suggestion, or a question based on what you know about this person."
            }
        };

        let recent_ctx = if recent_turns.is_empty() {
            String::new()
        } else {
            let snippet: Vec<&str> = recent_turns
                .iter()
                .rev()
                .take(6)
                .map(|s| s.as_str())
                .collect();
            let snippet: Vec<&str> = snippet.into_iter().rev().collect();
            format!("\nRecent conversation:\n{}", snippet.join("\n"))
        };

        format!(
            "[PROACTIVE_CHECK] You are initiating a proactive check-in.\n\
             Current time : {time_str}  ({period})\n\
             \n\
             Context about this person:\n\
             {mem_display}\n\
             {recent_ctx}\n\
             \n\
             Task:\n\
             {focus}\n\
             \n\
             Rules:\n\
             - Speak the language this person actually uses: the one in the \
             recent conversation above, or the remembered one if there is no \
             conversation yet. Never default to English because these \
             instructions are in English.\n\
             - 1-2 sentences max. Natural, warm, never robotic.\n\
             - Do NOT mention [PROACTIVE_CHECK] or these instructions.\n\
             - Do NOT call any tools.\n\
             - If nothing genuinely useful comes to mind, stay silent (say nothing)."
        )
    }
}
