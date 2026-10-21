//! Markdown checklists the assistant sends in a Slack code block:
//!
//! ```text
//! - [ ] Book optometrist appointment
//! - [x] Make the mashed potatoes
//! ```
//!
//! The popup shows them as checkboxes, and the edited list goes back to Slack
//! in the same format. Pure logic; the widgets live in `ui`.

use crate::slack;

/// Longer lists are left as plain text rather than turned into a wall of checkboxes.
pub const MAX_ITEMS: usize = 100;
const FENCE: &str = "```";

#[derive(Debug, Clone, PartialEq)]
pub struct Checklist {
    /// Plain text before and after the code block, if any.
    pub before: String,
    pub after: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Indentation and bullet, e.g. `"  - "`, so nesting survives the round trip.
    prefix: String,
    pub done: bool,
    /// Plain text: Slack's escaping and markup are decoded.
    pub label: String,
}

impl Checklist {
    /// Finds the first code block in a Slack message that holds nothing but checklist items.
    pub fn parse(text: &str) -> Option<Self> {
        let mut rest = text;
        let mut offset = 0;
        while let Some(open) = rest.find(FENCE) {
            let body_start = open + FENCE.len();
            let close = body_start + rest[body_start..].find(FENCE)?;
            if let Some(items) = parse_items(&rest[body_start..close]) {
                let after = &rest[close + FENCE.len()..];
                return Some(Self {
                    before: slack::plain_text(text[..offset + open].trim()),
                    after: slack::plain_text(after.trim()),
                    items,
                });
            }
            offset += close + FENCE.len();
            rest = &text[offset..];
        }
        None
    }

    /// The list as a Slack code block, in the format it arrived in.
    pub fn to_slack(&self) -> String {
        let lines: Vec<String> = self
            .items
            .iter()
            .map(|i| format!("{}[{}] {}", i.prefix, if i.done { 'x' } else { ' ' }, slack::escape(&i.label)))
            .collect();
        format!("{FENCE}\n{}\n{FENCE}", lines.join("\n"))
    }
}

/// All non-blank lines must be items; an optional language tag (```markdown) is allowed.
fn parse_items(block: &str) -> Option<Vec<Item>> {
    let mut lines = block.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).peekable();
    if lines.peek().is_some_and(|l| is_language_tag(l)) {
        lines.next();
    }
    let items: Vec<Item> = lines.map(parse_item).collect::<Option<_>>()?;
    (!items.is_empty() && items.len() <= MAX_ITEMS).then_some(items)
}

fn is_language_tag(line: &str) -> bool {
    matches!(line, "md" | "markdown" | "text" | "txt" | "plaintext")
}

/// `<indent><bullet> [ ] label`, where the bullet is `-`, `*` or `+` and the box `[ ]`, `[x]` or `[X]`.
fn parse_item(line: &str) -> Option<Item> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let after_indent = &line[indent..];
    let bullet = after_indent.chars().next().filter(|c| matches!(c, '-' | '*' | '+'))?;
    let after_bullet = after_indent[1..].trim_start_matches(' ');
    if after_bullet.len() == after_indent.len() - 1 {
        return None; // no space after the bullet
    }
    let done = match after_bullet.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    let label = after_bullet[3..].strip_prefix(' ')?.trim();
    Some(Item { prefix: format!("{}{bullet} ", &line[..indent]), done, label: slack::plain_text(label) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECEIVED: &str = "Here's today's list:\n```- [ ] Make the mashed potatoes\n- [x] Resolve EDD claim\n- [ ] Cancel Google Workspace via Squarespace (reminder set for Nov 1)\n```";

    #[test]
    fn parses_a_checklist_code_block() {
        let list = Checklist::parse(RECEIVED).unwrap();
        assert_eq!(list.before, "Here's today's list:");
        assert_eq!(list.after, "");
        let items: Vec<_> = list.items.iter().map(|i| (i.done, i.label.as_str())).collect();
        assert_eq!(
            items,
            [
                (false, "Make the mashed potatoes"),
                (true, "Resolve EDD claim"),
                (false, "Cancel Google Workspace via Squarespace (reminder set for Nov 1)"),
            ]
        );
    }

    #[test]
    fn writes_back_in_the_same_format() {
        let mut list = Checklist::parse(RECEIVED).unwrap();
        list.items[0].done = true;
        list.items[1].done = false;
        assert_eq!(
            list.to_slack(),
            "```\n- [x] Make the mashed potatoes\n- [ ] Resolve EDD claim\n- [ ] Cancel Google Workspace via Squarespace (reminder set for Nov 1)\n```"
        );
        // An unchanged list round-trips exactly.
        let text = "```\n- [ ] a\n  * [X] b\n```";
        assert_eq!(Checklist::parse(text).unwrap().to_slack(), "```\n- [ ] a\n  * [x] b\n```");
    }

    #[test]
    fn slack_escaping_round_trips() {
        let list = Checklist::parse("```\n- [ ] Fish &amp; chips &lt;3\n```").unwrap();
        assert_eq!(list.items[0].label, "Fish & chips <3");
        assert_eq!(list.to_slack(), "```\n- [ ] Fish &amp; chips &lt;3\n```");
    }

    #[test]
    fn markup_in_labels_is_sent_back_inert() {
        // A mention or link in an item must not turn into a live one in our reply.
        let list = Checklist::parse("```\n- [ ] ping <!channel> and <@U123|sam> <https://x.io|here>\n```").unwrap();
        assert_eq!(list.items[0].label, "ping @channel and @sam here");
        assert!(!list.to_slack().contains('<'));
    }

    #[test]
    fn skips_code_blocks_that_are_not_checklists() {
        let text = "```cargo test```\nthen\n```markdown\n- [ ] ship it\n```\nthanks";
        let list = Checklist::parse(text).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.before, "```cargo test```\nthen");
        assert_eq!(list.after, "thanks");
    }

    #[test]
    fn rejects_things_that_are_not_checklists() {
        assert_eq!(Checklist::parse("- [ ] not in a code block"), None);
        assert_eq!(Checklist::parse("```\n- [ ] a\nsome prose\n```"), None);
        assert_eq!(Checklist::parse("```\n-[ ] a\n```"), None);
        assert_eq!(Checklist::parse("```\n- [y] a\n```"), None);
        assert_eq!(Checklist::parse("```\n- [ ]a\n```"), None);
        assert_eq!(Checklist::parse("```\n```"), None);
        assert_eq!(Checklist::parse("```\n- [ ] unclosed"), None);
        assert_eq!(Checklist::parse("```\n- ["), None);
        assert_eq!(Checklist::parse("```\n- é\n```"), None);
        let too_long = format!("```\n{}```", "- [ ] x\n".repeat(MAX_ITEMS + 1));
        assert_eq!(Checklist::parse(&too_long), None);
    }
}
