/// Text worki fills into an agent prompt's `{name}` placeholder.
pub struct Part {
    pub name: &'static str,
    pub label: &'static str,
    pub text: String,
}

/// Fills `template`'s placeholders with `parts` and ends with `answer_rule`, which tells the
/// agent how to answer so worki can read it. A template that uses none of the placeholders (one
/// written before they existed) gets every part appended under its label instead.
pub fn fill(template: &str, parts: &[Part], answer_rule: &str) -> String {
    let template = template.trim();
    let placeholder = |p: &Part| format!("{{{}}}", p.name);
    let body = if parts.iter().any(|p| template.contains(&placeholder(p))) {
        parts.iter().fold(template.to_owned(), |text, p| {
            text.replace(&placeholder(p), p.text.trim_end())
        })
    } else {
        parts.iter().fold(template.to_owned(), |text, p| {
            format!("{text}\n\n{}:\n{}", p.label, p.text.trim_end())
        })
    };
    format!("{body}\n\nDon't use any tools; everything you need is above. {answer_rule}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> Vec<Part> {
        vec![
            Part {
                name: "log",
                label: "Recent commit messages",
                text: "- Add a\n".into(),
            },
            Part {
                name: "diff",
                label: "Changes",
                text: "+b\n".into(),
            },
        ]
    }

    #[test]
    fn fills_placeholders_and_appends_the_answer_rule() {
        assert_eq!(
            fill("Like {log}, describe {diff} ({x})", &parts(), "Only the message."),
            "Like - Add a, describe +b ({x})\n\nDon't use any tools; everything you need is above. Only the message."
        );
    }

    #[test]
    fn appends_the_parts_to_a_prompt_without_placeholders() {
        assert_eq!(
            fill("Describe it.", &parts(), "Only the message."),
            "Describe it.\n\nRecent commit messages:\n- Add a\n\nChanges:\n+b\n\nDon't use any tools; everything you need is above. Only the message."
        );
    }
}
