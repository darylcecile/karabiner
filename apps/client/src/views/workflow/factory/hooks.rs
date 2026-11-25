//! Text forms for stage hooks and gates: one command per line (`executable arg "quoted arg"`),
//! gates as `approval: prompt` or `command: executable args`. Ids are kept by position so
//! editing a line does not churn references in run history.

use protocol::{CommandDefinition, GateDefinition, GateKind};

pub fn commands_text(commands: Option<&Vec<CommandDefinition>>) -> String {
    commands.map_or_else(String::new, |commands| commands.iter().map(command_line).collect::<Vec<_>>().join("\n"))
}

pub fn parse_commands(text: &str, previous: Option<&Vec<CommandDefinition>>, prefix: &str) -> Option<Vec<CommandDefinition>> {
    let commands: Vec<CommandDefinition> = lines(text).enumerate().filter_map(|(index, line)| {
        let existing = previous.and_then(|previous| previous.get(index));
        parse_command(line, existing, &format!("{prefix}-{}", index + 1))
    }).collect();
    (!commands.is_empty()).then_some(commands)
}

pub fn gates_text(gates: Option<&Vec<GateDefinition>>) -> String {
    gates.map_or_else(String::new, |gates| gates.iter().map(|gate| match (&gate.kind, &gate.command, &gate.prompt) {
        (GateKind::Command, Some(command), _) => format!("command: {}", command_line(command)),
        (_, _, Some(prompt)) => format!("approval: {prompt}"),
        (kind, _, _) => format!("{}:", kind.as_str()),
    }).collect::<Vec<_>>().join("\n"))
}

pub fn parse_gates(text: &str, previous: Option<&Vec<GateDefinition>>, prefix: &str) -> Option<Vec<GateDefinition>> {
    let gates: Vec<GateDefinition> = lines(text).enumerate().map(|(index, line)| {
        let id = previous.and_then(|previous| previous.get(index)).map_or_else(|| format!("{prefix}-{}", index + 1), |gate| gate.id.clone());
        let mut gate = previous.and_then(|previous| previous.get(index)).cloned().unwrap_or(GateDefinition {
            id: id.clone(), kind: GateKind::Approval, name: None, command: None, prompt: None, extra: Default::default(),
        });
        match line.split_once(':').map(|(kind, rest)| (kind.trim(), rest.trim())) {
            Some(("command", rest)) => {
                gate.kind = GateKind::Command;
                gate.command = parse_command(rest, gate.command.as_ref(), &format!("{id}-command"));
                gate.prompt = None;
            }
            Some(("approval", rest)) => { gate.kind = GateKind::Approval; gate.prompt = Some(rest.into()); gate.command = None }
            _ => { gate.kind = GateKind::Approval; gate.prompt = Some(line.into()); gate.command = None }
        }
        gate
    }).collect();
    (!gates.is_empty()).then_some(gates)
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|line| !line.is_empty())
}

fn parse_command(line: &str, existing: Option<&CommandDefinition>, id: &str) -> Option<CommandDefinition> {
    let mut words = split_words(line).into_iter();
    let executable = words.next()?;
    let mut command = existing.cloned().unwrap_or_else(|| CommandDefinition { id: id.into(), ..Default::default() });
    command.executable = executable;
    command.args = words.collect();
    Some(command)
}

fn command_line(command: &CommandDefinition) -> String {
    std::iter::once(&command.executable).chain(&command.args).map(|word| quote(word)).collect::<Vec<_>>().join(" ")
}

fn quote(word: &str) -> String {
    if !word.is_empty() && !word.contains([' ', '"', '\t']) { word.into() } else { format!("\"{}\"", word.replace('"', "\\\"")) }
}

/// Whitespace split honouring double quotes and `\"`.
fn split_words(line: &str) -> Vec<String> {
    let (mut words, mut current, mut quoted, mut started) = (Vec::new(), String::new(), false, false);
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if quoted && chars.peek() == Some(&'"') => { current.push('"'); chars.next(); }
            '"' => { quoted = !quoted; started = true }
            ch if ch.is_whitespace() && !quoted => {
                if started { words.push(std::mem::take(&mut current)); started = false }
            }
            ch => { current.push(ch); started = true }
        }
    }
    if started { words.push(current) }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_with_quotes_and_keep_ids() {
        let parsed = parse_commands("bun test \"a b\"\n\n cargo check ", None, "cmd").unwrap();
        assert_eq!(parsed[0].args, ["test", "a b"]);
        assert_eq!(parsed[1].id, "cmd-2");
        assert_eq!(commands_text(Some(&parsed)), "bun test \"a b\"\ncargo check");
        let again = parse_commands("bun lint", Some(&parsed), "cmd").unwrap();
        assert_eq!(again[0].id, "cmd-1");
    }

    #[test]
    fn gates_parse_both_kinds() {
        let gates = parse_gates("approval: Ship it?\ncommand: bun test", None, "gate").unwrap();
        assert_eq!(gates[0].prompt.as_deref(), Some("Ship it?"));
        assert_eq!(gates[1].command.as_ref().unwrap().executable, "bun");
        assert_eq!(gates_text(Some(&gates)), "approval: Ship it?\ncommand: bun test");
    }
}
