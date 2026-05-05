use crate::config::ResolvedConfig;
use rand::Rng;

#[derive(Clone)]
pub enum UserAgentMode {
    /// Single fixed User-Agent string
    Single(String),
    /// Rotate through the built-in browser-like User-Agent list
    RotateBuiltIn,
    /// Rotate through a list of User-Agents loaded from config
    RotateList(Vec<String>),
}

fn default_product_user_agent() -> String {
    "warmer/0.1.2 (+https://abh.ai/warmer)".to_string()
}

pub fn build_user_agent_mode(resolved: &ResolvedConfig) -> UserAgentMode {
    // 1. Single User-Agent from config
    if let Some(ref ua) = resolved.user_agent {
        return UserAgentMode::Single(ua.clone());
    }

    // 2. User-Agent list from config (custom list in .toml)
    if !resolved.user_agent_list.is_empty() {
        if resolved.user_agent_list.len() == 1 {
            return UserAgentMode::Single(resolved.user_agent_list[0].clone());
        } else {
            return UserAgentMode::RotateList(resolved.user_agent_list.clone());
        }
    }

    // 3. -a/--anonymize: rotate through built-in list in code
    if resolved.anonymize {
        UserAgentMode::RotateBuiltIn
    } else {
        // 4. Default: single product User-Agent
        UserAgentMode::Single(default_product_user_agent())
    }
}

pub fn get_user_agent(mode: &UserAgentMode) -> String {
    match mode {
        UserAgentMode::Single(ua) => ua.clone(),
        UserAgentMode::RotateBuiltIn => {
            let user_agents = [
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 OPR/129.0.0.0",
            ];
            let mut rng = rand::rng();
            user_agents[rng.random_range(0..user_agents.len())].to_string()
        }
        UserAgentMode::RotateList(list) => {
            if list.is_empty() {
                return default_product_user_agent();
            }
            let mut rng = rand::rng();
            list[rng.random_range(0..list.len())].clone()
        }
    }
}
