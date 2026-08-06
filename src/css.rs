use crate::config::UserMap;

/// Build the full injected stylesheet for transparent canvas + per-user avatars.
pub fn generate_overlay_css(users: &UserMap) -> String {
    let mut css = String::with_capacity(2048 + users.len() * 512);

    css.push_str(
        r#"/* Discord Overlay — transparent canvas + custom avatars */
/*
 * Selectors target Streamkit's stable, unhashed class names:
 *   <li class="Voice_voiceState__… [wrapper_speaking] voice_state" data-userid="…">
 *     <img class="Voice_avatar__… [Voice_avatarSpeaking__…] voice_avatar">
 * `wrapper_speaking` (on the li) and `voice_avatar` (on the img) survive
 * Streamkit rebuilds; the CSS-module names carry a build hash and do not.
 */
/* Base transparent canvas settings */
body {
    margin: 0;
    overflow: hidden;
    background: transparent !important;
}

/* Keep voice states as a compact inline grid */
li.voice_state {
    position: static;
    display: inline-grid;
}

/* Hide default avatars for all users unless overridden */
li > img.voice_avatar {
    display: none;
}

@keyframes speak-bounce {
    0%, 100% { transform: translateY(0) scale(1); }
    25%      { transform: translateY(-8px) scale(1.03); }
    50%      { transform: translateY(0) scale(1); }
    75%      { transform: translateY(-4px) scale(1.02); }
}

"#,
    );

    for (user_id, avatar) in users {
        let idle = escape_css_url(&avatar.idle_url);
        let speaking = escape_css_url(&avatar.speaking_url);
        let id = escape_css_ident(user_id);
        let order = avatar.order;

        css.push_str(&format!(
            r#"/* User {user_id} */
li[data-userid="{id}"] > img.voice_avatar {{
    display: block !important;
    content: url("{idle}");
    width: auto !important;
    height: auto !important;
    border-radius: 0 !important;
    filter: brightness(60%);
    transition: filter 0.2s ease, transform 0.2s ease;
}}

/* Configured render order: lower values appear first. */
li[data-userid="{id}"] {{
    order: {order} !important;
}}

/* Speaking: the `wrapper_speaking` class lands on the li, the hashed
 * `Voice_avatarSpeaking…` one on the img — either is enough to match. */
li[data-userid="{id}"].wrapper_speaking > img.voice_avatar,
li[data-userid="{id}"] > img.voice_avatar[class*="avatarSpeaking"] {{
    content: url("{speaking}") !important;
    filter: brightness(100%);
    animation: speak-bounce 0.6s ease-in-out infinite;
}}

"#
        ));
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
