use crate::config::UserMap;
use crate::web::{OVERLAY_CSS, USER_CSS_TEMPLATE};

/// Build the full injected stylesheet for transparent canvas + per-user avatars.
pub fn generate_overlay_css(users: &UserMap) -> String {
    let mut css = String::with_capacity(OVERLAY_CSS.len() + users.len() * 512);
    css.push_str(OVERLAY_CSS);

    // `UserMap` is a HashMap, so iterating it directly reshuffles the rules on
    // every reload. Emit by (order, id) instead: the stylesheet is stable
    // across restarts and reads in the same sequence the overlay renders.
    let mut ordered: Vec<_> = users.iter().collect();
    ordered.sort_by_key(|(user_id, avatar)| (avatar.order, *user_id));

    for (user_id, avatar) in ordered {
        let idle = escape_css_url(&avatar.idle_url);
        let speaking = escape_css_url(&avatar.speaking_url);
        let id = escape_css_ident(user_id);
        css.push_str(
            &USER_CSS_TEMPLATE
                .replace("{{USER_ID}}", &id)
                .replace("{{IDLE_URL}}", &idle)
                .replace("{{SPEAKING_URL}}", &speaking)
                .replace("{{ORDER}}", &avatar.order.to_string()),
        );
    }

    css
}

/// Minimal escaping for URLs embedded in `url("...")`.
fn escape_css_url(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape characters that could break attribute selectors.
fn escape_css_ident(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' | '\\' => format!("\\{c}"),
            c if c.is_control() => String::new(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AvatarOverride;
    use std::collections::HashMap;

    #[test]
    fn generates_per_user_rules() {
        let mut users = HashMap::new();
        users.insert(
            "123456789".into(),
            AvatarOverride {
                idle_url: "https://cdn.example/idle.png".into(),
                speaking_url: "https://cdn.example/speak.png".into(),
                order: 2,
            },
        );

        let css = generate_overlay_css(&users);
        assert!(css.contains("data-userid=\"123456789\""));
        assert!(css.contains("https://cdn.example/idle.png"));
        assert!(css.contains("https://cdn.example/speak.png"));
        assert!(css.contains("order: 2 !important"));
        assert!(css.contains("speak-bounce"));
        assert!(css.contains("background: transparent !important"));
    }

    /// `order` is inert unless the parent is a flex/grid container, and
    /// Streamkit's `ul.voice_states` is a plain block list.
    #[test]
    fn voice_state_list_is_a_flex_container_so_order_applies() {
        let css = generate_overlay_css(&HashMap::new());
        assert!(css.contains("ul.voice_states"));
        assert!(css.contains("display: flex"));
    }

    #[test]
    fn rules_are_emitted_in_configured_order() {
        let avatar = |order| AvatarOverride {
            idle_url: "idle.png".into(),
            speaking_url: "speak.png".into(),
            order,
        };
        let mut users = HashMap::new();
        users.insert("aaa".to_string(), avatar(9));
        users.insert("zzz".to_string(), avatar(-1));

        let css = generate_overlay_css(&users);
        assert!(css.find("/* User zzz */").unwrap() < css.find("/* User aaa */").unwrap());
    }

    /// The speaking state must key off Streamkit's unhashed class names:
    /// `wrapper_speaking` sits on the `li`, not on the avatar `img`.
    #[test]
    fn speaking_rule_targets_the_stable_class_names() {
        let mut users = HashMap::new();
        users.insert(
            "42".into(),
            AvatarOverride {
                idle_url: "idle.png".into(),
                speaking_url: "speak.png".into(),
                order: 1,
            },
        );

        let css = generate_overlay_css(&users);
        assert!(css.contains(r#"li[data-userid="42"].wrapper_speaking > img.voice_avatar"#));
        assert!(css.contains(r#"img.voice_avatar[class*="avatarSpeaking"]"#));
        // The old selector looked for the speaking class on the li, where it
        // never appears — that is why the image never swapped.
        assert!(!css.contains(r#"[class*="Voice_avatarSpeaking"] >"#));
    }
}
