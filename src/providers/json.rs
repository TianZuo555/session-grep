use serde_json::Value;

use crate::model::{Message, Role, one_line};

/// Pull readable text out of a message `content` field.
///
/// Every provider uses either a plain string or a list of typed parts, so one
/// walker covers all of them.
pub fn content_messages(role: Role, content: &Value, out: &mut Vec<Message>) {
    match content {
        Value::String(text) => push_text(role, text, out),
        Value::Array(parts) => {
            for part in parts {
                part_message(role, part, out);
            }
        }
        _ => {}
    }
}

fn part_message(role: Role, part: &Value, out: &mut Vec<Message>) {
    let field = |names: &[&str]| names.iter().find_map(|name| part.get(*name));
    let text_of = |value: Option<&Value>| match value {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    match part["type"].as_str().unwrap_or("") {
        "text" | "input_text" | "output_text" => push_text(role, &text_of(field(&["text"])), out),
        "thinking" | "reasoning" | "summary_text" => {
            push_text(Role::Thinking, &text_of(field(&["thinking", "text"])), out)
        }
        "toolCall" | "tool_use" | "tool-call" | "function_call" | "custom_tool_call" => {
            let name = text_of(field(&["name", "toolName"]));
            let args = text_of(field(&["arguments", "input", "args"]));
            out.push(Message::new(Role::Tool, format!("{name}({args})")));
        }
        "tool_result" | "tool-result" | "function_call_output" | "custom_tool_call_output" => {
            let body = text_of(field(&["output", "result", "content"]));
            push_text(Role::Tool, &body, out);
        }
        _ => {}
    }
}

fn push_text(role: Role, text: &str, out: &mut Vec<Message>) {
    let (role, text) = match role {
        Role::User => classify_user(text),
        _ => (role, text.trim()),
    };
    if !text.is_empty() {
        out.push(Message::new(role, text));
    }
}

/// Harnesses send injected context with the user role. Cursor wraps the real
/// prompt in `<user_query>`. Codex sends `# AGENTS.md instructions`. Most of the
/// others open the block with an XML tag.
fn classify_user(text: &str) -> (Role, &str) {
    let text = text.trim();
    if let Some(start) = text.find("<user_query>") {
        let inner = &text[start + "<user_query>".len()..];
        let end = inner.find("</user_query>").unwrap_or(inner.len());
        return (Role::User, inner[..end].trim());
    }
    if text.starts_with('<') || text.starts_with("# AGENTS.md instructions") {
        return (Role::System, text);
    }
    (Role::User, text)
}

/// The first user message, for the list title.
pub fn title_of(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| one_line(&m.text, 160))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(role: Role, content: Value) -> Vec<Message> {
        let mut out = Vec::new();
        content_messages(role, &content, &mut out);
        out
    }

    #[test]
    fn string_content_is_one_message() {
        assert_eq!(
            parse(Role::Assistant, json!(" hi ")),
            [Message::new(Role::Assistant, "hi")]
        );
    }

    #[test]
    fn typed_parts_map_to_roles() {
        let out = parse(
            Role::Assistant,
            json!([
                {"type": "thinking", "thinking": "plan"},
                {"type": "text", "text": "answer"},
                {"type": "tool_use", "name": "bash", "input": {"cmd": "ls"}},
            ]),
        );
        assert_eq!(
            out,
            [
                Message::new(Role::Thinking, "plan"),
                Message::new(Role::Assistant, "answer"),
                Message::new(Role::Tool, r#"bash({"cmd":"ls"})"#),
            ]
        );
    }

    #[test]
    fn injected_context_is_system() {
        let out = parse(
            Role::User,
            json!("# AGENTS.md instructions for /w <INSTRUCTIONS>"),
        );
        assert_eq!(out[0].role, Role::System);
        let out = parse(
            Role::User,
            json!("<environment_context>cwd</environment_context>"),
        );
        assert_eq!(out[0].role, Role::System);
    }

    #[test]
    fn cursor_user_query_is_unwrapped() {
        let out = parse(
            Role::User,
            json!([{"type": "text", "text": "<timestamp>x</timestamp>\n<user_query>\nfix it\n</user_query>"}]),
        );
        assert_eq!(out, [Message::new(Role::User, "fix it")]);
    }

    #[test]
    fn title_skips_context() {
        let out = parse(
            Role::User,
            json!([
                {"type": "text", "text": "<user_info>os</user_info>"},
                {"type": "text", "text": "real question"},
            ]),
        );
        assert_eq!(title_of(&out), "real question");
    }
}
