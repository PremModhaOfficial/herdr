use std::io;
use std::path::Path;

use indexmap::IndexMap;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;

use super::command::hook_command;
use super::config_edit::hook_command_variants;

type Object = IndexMap<String, OrderedValue>;

#[derive(Serialize)]
#[serde(untagged)]
enum OrderedValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<OrderedValue>),
    Object(Object),
}

impl<'de> Deserialize<'de> for OrderedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(OrderedValueVisitor)
    }
}

struct OrderedValueVisitor;

impl<'de> Visitor<'de> for OrderedValueVisitor {
    type Value = OrderedValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(OrderedValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(OrderedValue::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(OrderedValue::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(OrderedValue::Number)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_string())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(OrderedValue::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(OrderedValue::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(OrderedValue::Null)
    }

    fn visit_seq<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut result = Vec::with_capacity(values.size_hint().unwrap_or(0));
        while let Some(value) = values.next_element()? {
            result.push(value);
        }
        Ok(OrderedValue::Array(result))
    }

    fn visit_map<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut result = Object::with_capacity(values.size_hint().unwrap_or(0));
        while let Some((key, value)) = values.next_entry()? {
            result.insert(key, value);
        }
        Ok(OrderedValue::Object(result))
    }
}

impl OrderedValue {
    fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[OrderedValue]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    fn as_array_mut(&mut self) -> Option<&mut Vec<OrderedValue>> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&OrderedValue> {
        match self {
            Self::Object(value) => value.get(key),
            _ => None,
        }
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut OrderedValue> {
        match self {
            Self::Object(value) => value.get_mut(key),
            _ => None,
        }
    }
}

pub(crate) fn update_claude_settings(
    content: &str,
    settings_path: &Path,
    hook_path: &Path,
) -> io::Result<String> {
    let trailing_newline = content.ends_with('\n');
    let mut settings = serde_json::from_str::<OrderedValue>(content).map_err(|err| {
        io::Error::other(format!(
            "failed to parse {}: {err}",
            settings_path.display()
        ))
    })?;

    let hooks = ensure_hooks_object(&mut settings, settings_path)?;
    remove_hook_commands(hooks, "PostToolUse", hook_path, Some("working"))?;
    remove_hook_commands(hooks, "PostToolUseFailure", hook_path, Some("working"))?;
    remove_hook_commands(hooks, "SubagentStop", hook_path, Some("working"))?;
    remove_hook_commands(hooks, "PermissionRequest", hook_path, Some("blocked"))?;
    remove_hook_commands(hooks, "SessionStart", hook_path, Some("idle"))?;
    remove_hook_commands(hooks, "UserPromptSubmit", hook_path, Some("working"))?;
    remove_hook_commands(hooks, "PreToolUse", hook_path, Some("working"))?;
    remove_hook_commands(hooks, "Stop", hook_path, Some("idle"))?;
    remove_hook_commands(hooks, "SessionEnd", hook_path, Some("release"))?;
    remove_hook_commands(hooks, "SessionStart", hook_path, Some("session"))?;
    ensure_command_hook(
        hooks,
        "SessionStart",
        hook_command(hook_path, Some("session")),
        10,
        Some("*"),
    )?;

    let mut updated = serde_json::to_string_pretty(&settings)?;
    if trailing_newline {
        updated.push('\n');
    }
    Ok(updated)
}

fn ensure_hooks_object<'a>(
    settings: &'a mut OrderedValue,
    settings_path: &Path,
) -> io::Result<&'a mut Object> {
    let root = settings.as_object_mut().ok_or_else(|| {
        io::Error::other(format!(
            "claude settings at {} must be a JSON object",
            settings_path.display()
        ))
    })?;

    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| OrderedValue::Object(Object::new()));
    hooks.as_object_mut().ok_or_else(|| {
        io::Error::other(format!(
            "claude settings hooks at {} must be a JSON object",
            settings_path.display()
        ))
    })
}

fn ensure_command_hook(
    hooks: &mut Object,
    event: &str,
    command: String,
    timeout: u64,
    matcher: Option<&str>,
) -> io::Result<()> {
    let entries = hooks
        .entry(event.to_string())
        .or_insert_with(|| OrderedValue::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| io::Error::other(format!("hook entries for {event} must be an array")))?;

    let already_installed = entries.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(OrderedValue::as_array)
            .is_some_and(|hook_entries| {
                hook_entries.iter().any(|hook| {
                    hook.get("type").and_then(OrderedValue::as_str) == Some("command")
                        && hook.get("command").and_then(OrderedValue::as_str)
                            == Some(command.as_str())
                })
            })
    });
    if already_installed {
        return Ok(());
    }

    let mut command_entry = Object::new();
    command_entry.insert(
        "type".to_string(),
        OrderedValue::String("command".to_string()),
    );
    command_entry.insert("command".to_string(), OrderedValue::String(command));
    command_entry.insert("timeout".to_string(), OrderedValue::Number(timeout.into()));

    let mut entry = Object::new();
    if let Some(matcher) = matcher {
        entry.insert(
            "matcher".to_string(),
            OrderedValue::String(matcher.to_string()),
        );
    }
    entry.insert(
        "hooks".to_string(),
        OrderedValue::Array(vec![OrderedValue::Object(command_entry)]),
    );
    entries.push(OrderedValue::Object(entry));
    Ok(())
}

fn remove_hook_commands(
    hooks: &mut Object,
    event: &str,
    hook_path: &Path,
    action: Option<&str>,
) -> io::Result<()> {
    for command in hook_command_variants(hook_path, action) {
        remove_command_hook(hooks, event, &command)?;
    }
    Ok(())
}

fn remove_command_hook(hooks: &mut Object, event: &str, command: &str) -> io::Result<()> {
    let Some(entries_value) = hooks.get_mut(event) else {
        return Ok(());
    };
    let entries = entries_value
        .as_array_mut()
        .ok_or_else(|| io::Error::other(format!("hook entries for {event} must be an array")))?;

    entries.retain_mut(|entry| {
        let Some(hook_entries) = entry.get_mut("hooks").and_then(OrderedValue::as_array_mut) else {
            return true;
        };
        hook_entries.retain(|hook| {
            !(hook.get("type").and_then(OrderedValue::as_str) == Some("command")
                && hook.get("command").and_then(OrderedValue::as_str) == Some(command))
        });
        !hook_entries.is_empty()
    });

    // SessionStart is always repopulated with the canonical hook after migration.
    // Keep its existing map slot so removing an obsolete entry does not move the
    // surviving event behind unrelated user hook events.
    if entries.is_empty() && event != "SessionStart" {
        hooks.shift_remove(event);
    }
    Ok(())
}
