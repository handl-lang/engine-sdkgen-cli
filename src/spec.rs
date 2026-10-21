//! The authored `*.clispec.yaml` spec format for the `cli` backend.
//!
//! A spec names a tool (`name`) and its command tree (`commands`), each
//! node carrying `about` docs, `options`, `positionals`, an optional
//! variadic `rest` tail, and nested `commands`. The top-level spec is
//! itself a command node (flattened), so a tool with no subcommands
//! declares its `options`/`positionals` at the root.
//!
//! ```yaml
//! name: gh
//! about: GitHub CLI
//! commands:
//!   pr:
//!     about: Manage pull requests
//!     commands:
//!       create:
//!         about: Create a pull request
//!         options:
//!           title: { flag: "--title", kind: string, doc: "PR title" }
//!           draft: { flag: "--draft", kind: flag }
//!         positionals: [{ name: branch, kind: string }]
//!         rest: { name: extras, kind: string }
//! ```

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Option kinds recognized by the spec.
pub const OPTION_KINDS: &[&str] = &["flag", "string", "int", "strings"];
/// Positional (and `rest` element) kinds recognized by the spec.
pub const POSITIONAL_KINDS: &[&str] = &["string", "int", "bool"];

/// A loaded `*.clispec.yaml` document.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CliSpec {
    /// Tool/binary name — the module name basis and the probed argv[0].
    pub name: String,
    /// Module doc.
    pub about: Option<String>,
    /// When present, the spec is overlaid on a live `--help` probe of
    /// `name` (`spec`-authored nodes win; see [`crate::probe`]).
    pub probe: Option<ProbeSpec>,
    /// The root command node — `commands`, `options`, `positionals`,
    /// `rest` all belong to the tool itself at the top level.
    #[serde(flatten)]
    pub root: CommandNode,
}

/// `probe:` settings — how deep and with which flag to walk `--help`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProbeSpec {
    /// The help flag appended to each probed argv (default `--help`).
    pub flag: Option<String>,
    /// How many subcommand levels below the root to probe (default 1).
    pub max_depth: Option<usize>,
}

/// One node of the command tree.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CommandNode {
    /// Doc for the emitted function.
    pub about: Option<String>,
    /// Long options, keyed by spec name.
    pub options: BTreeMap<String, OptionSpec>,
    /// Positional arguments, in argv order.
    pub positionals: Vec<PositionalSpec>,
    /// Optional variadic tail (`args...`).
    pub rest: Option<PositionalSpec>,
    /// Nested subcommands.
    pub commands: BTreeMap<String, CommandNode>,
}

/// A named option (`--title`), keyed under `options:`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OptionSpec {
    /// The flag spelling handed to the tool (`--title`, `-t`). Must
    /// start with `-` and contain no `:` (the IR descriptor is
    /// colon-separated).
    pub flag: String,
    /// `flag` | `string` | `int` | `strings` (repeatable).
    #[serde(default = "default_option_kind")]
    pub kind: String,
    /// Doc for the option.
    pub doc: Option<String>,
}

impl Default for OptionSpec {
    fn default() -> Self {
        Self {
            flag: String::new(),
            kind: default_option_kind(),
            doc: None,
        }
    }
}

/// A positional argument or the variadic `rest` tail.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PositionalSpec {
    /// Argument name — used for the emitted port arg and the
    /// `pos:`/`rest:` IR descriptor.
    pub name: String,
    /// `string` | `int` | `bool`.
    #[serde(default = "default_positional_kind")]
    pub kind: String,
    /// Doc for the argument.
    pub doc: Option<String>,
}

impl Default for PositionalSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: default_positional_kind(),
            doc: None,
        }
    }
}

fn default_option_kind() -> String {
    "flag".to_string()
}

fn default_positional_kind() -> String {
    "string".to_string()
}

impl CliSpec {
    /// Load and validate a `*.clispec.yaml` file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
        let spec: CliSpec =
            serde_yaml::from_str(&s).map_err(|e| format!("failed to parse spec: {e}"))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Validate the spec — name, kinds, flag spellings, and per-command
    /// uniqueness. Errors carry the dotted command path.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("spec is missing `name`".to_string());
        }
        self.root.validate_at(&self.name)
    }
}

impl CommandNode {
    /// Validate this node as the command at dotted path `path`.
    fn validate_at(&self, path: &str) -> Result<(), String> {
        let mut flags = std::collections::HashSet::new();
        for (name, opt) in &self.options {
            if name.is_empty() || name.contains(':') {
                return Err(format!("{path}: invalid option name `{name}`"));
            }
            if !opt.flag.starts_with('-') {
                return Err(format!(
                    "{path}: option `{name}` flag `{}` must start with `-`",
                    opt.flag
                ));
            }
            if opt.flag.contains(':') {
                return Err(format!(
                    "{path}: option `{name}` flag `{}` must not contain `:`",
                    opt.flag
                ));
            }
            if !OPTION_KINDS.contains(&opt.kind.as_str()) {
                return Err(format!(
                    "{path}: option `{name}` has unknown kind `{}` (expected one of {})",
                    opt.kind,
                    OPTION_KINDS.join(", ")
                ));
            }
            if !flags.insert(opt.flag.as_str()) {
                return Err(format!("{path}: duplicate flag spelling `{}`", opt.flag));
            }
        }
        let mut names = std::collections::HashSet::new();
        for pos in &self.positionals {
            validate_positional(path, pos)?;
            if !names.insert(pos.name.as_str()) {
                return Err(format!("{path}: duplicate positional `{}`", pos.name));
            }
        }
        if let Some(rest) = &self.rest {
            validate_positional(path, rest)?;
        }
        for (name, sub) in &self.commands {
            if name.is_empty() {
                return Err(format!("{path}: empty subcommand name"));
            }
            sub.validate_at(&format!("{path}.{name}"))?;
        }
        Ok(())
    }
}

fn validate_positional(path: &str, pos: &PositionalSpec) -> Result<(), String> {
    if pos.name.is_empty() || pos.name.contains(':') {
        return Err(format!("{path}: invalid positional name `{}`", pos.name));
    }
    if !POSITIONAL_KINDS.contains(&pos.kind.as_str()) {
        return Err(format!(
            "{path}: positional `{}` has unknown kind `{}` (expected one of {})",
            pos.name,
            pos.kind,
            POSITIONAL_KINDS.join(", ")
        ));
    }
    Ok(())
}
