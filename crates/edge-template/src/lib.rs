//! The placeholder language of custom G-code: the machine's start, end and
//! layer-change snippets are templates, and this fills them in.
//!
//! Covered, after the reference's PlaceholderParser: `{expression}`,
//! `{if}` / `{elsif}` / `{else}` / `{endif}`, and the older `[variable]`
//! form. Expressions have numbers, strings, booleans, variables with an
//! optional `[index]`, arithmetic, comparison, `and` / `or` / `not`, a
//! ternary, and a few functions.
//!
//! Between braces there may also be a script, as newer tool-change
//! templates have: statements separated by `;` that declare variables
//! (`local`, `global`), set them, branch (`if … then … elsif … else …
//! endif`) and write out the value of every statement that is just an
//! expression.
//!
//! Not covered: regular-expression matching, list literals and `repeat`.
//! A template using them is an error, never a silent skip.

mod expression;

use std::collections::HashMap;

pub use expression::Value;

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct TemplateError {
    /// Line of the template the problem is on, counting from 1.
    pub line: usize,
    pub message: String,
}

/// What a template can refer to by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variables {
    values: HashMap<String, Variable>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Variable {
    One(Value),
    /// One entry per extruder or filament.
    Many(Vec<Value>),
}

impl Variable {
    /// The value; `index` picks an entry of a list. A list read without an
    /// index gives its first entry, as the reference does for the older
    /// `[variable]` form, and so does one read past its end or before its
    /// start (PlaceholderParser.cpp, `vector_element_to_expr`): Bambu's
    /// filament change asks for `long_retractions_when_cut[previous_extruder]`
    /// of a one-nozzle machine with any filament, and with -1 before the
    /// first is loaded. An empty list has nothing to give.
    pub(crate) fn read(&self, name: &str, index: Option<i64>) -> Result<Value, String> {
        match (self, index) {
            (Variable::One(value), None) => Ok(value.clone()),
            (Variable::One(_), Some(_)) => Err(format!(
                "\"{name}\" is a single value and cannot be indexed"
            )),
            (Variable::Many(values), index) => {
                let index = usize::try_from(index.unwrap_or(0))
                    .ok()
                    .filter(|index| *index < values.len())
                    .unwrap_or(0);
                values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("\"{name}\" is an empty list and cannot be indexed"))
            }
        }
    }
}

impl Variables {
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.values.insert(name.into(), Variable::One(value));
    }

    pub fn set_list(&mut self, name: impl Into<String>, values: Vec<Value>) {
        self.values.insert(name.into(), Variable::Many(values));
    }

    /// Takes a variable away, so that a template naming it is refused.
    pub fn remove(&mut self, name: &str) {
        self.values.remove(name);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub(crate) fn variable(&self, name: &str) -> Option<&Variable> {
        self.values.get(name)
    }

    /// The value of a single variable, or of a list's first entry.
    pub fn value(&self, name: &str) -> Option<Value> {
        self.get(name, None).ok()
    }

    /// The entries of a list.
    pub fn list(&self, name: &str) -> Option<&[Value]> {
        match self.values.get(name)? {
            Variable::Many(values) => Some(values),
            Variable::One(_) => None,
        }
    }

    fn get(&self, name: &str, index: Option<i64>) -> Result<Value, String> {
        self.values
            .get(name)
            .ok_or_else(|| format!("unknown variable \"{name}\""))?
            .read(name, index)
    }
}

/// Fills in a template, discarding whatever it sets.
pub fn expand(template: &str, variables: &Variables) -> Result<String, TemplateError> {
    expand_over(template, variables, &mut expression::Overlay::default())
}

/// Fills in a template and keeps what it sets: new values of variables
/// that were given, and variables it declares `global`. What it declares
/// `local` ends with it.
pub fn expand_mut(template: &str, variables: &mut Variables) -> Result<String, TemplateError> {
    let mut overlay = expression::Overlay::default();
    let text = expand_over(template, variables, &mut overlay)?;
    for (name, variable) in overlay.values {
        if !overlay.locals.contains(&name) {
            variables.values.insert(name, variable);
        }
    }
    Ok(text)
}

/// Fills in a template. Text outside braces and brackets is copied as it
/// is, including the rest of a line that held only a directive, which is
/// what the reference does too.
fn expand_over(
    template: &str,
    variables: &Variables,
    overlay: &mut expression::Overlay,
) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
    let read = |overlay: &expression::Overlay, name: &str| match overlay.values.get(name) {
        Some(variable) => variable.read(name, None),
        None => variables.get(name, None),
    };
    // One entry per open `{if}`: whether its text is being emitted, whether
    // any branch has been taken yet, and whether the enclosing text is live.
    struct Branch {
        emitting: bool,
        taken: bool,
        outer: bool,
        /// Where the `{if}` is, for reporting one left open.
        line: usize,
    }
    let mut branches: Vec<Branch> = Vec::new();
    let mut rest = template;
    let mut line = 1;

    while let Some(start) = rest.find(['{', '[']) {
        let live = branches.last().is_none_or(|branch| branch.emitting);
        let (text, tag) = rest.split_at(start);
        if live {
            out.push_str(text);
        }
        line += text.matches('\n').count();
        let fail = |message: String| TemplateError { line, message };

        let close = if tag.starts_with('{') { '}' } else { ']' };
        let end = closing(tag, close)
            .ok_or_else(|| fail(format!("\"{}\" is never closed", &tag[..1])))?;
        let body = &tag[1..end];
        rest = &tag[end + 1..];

        if close == ']' {
            // The older form names one variable, nothing more.
            if live {
                let name = body.trim();
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(fail(format!("\"[{body}]\" is not a variable name")));
                }
                // A number as the reference writes a setting's value out
                // (Config.hpp `ConfigOptionFloat::serialize`, a stream's
                // six significant digits).
                out.push_str(&read(overlay, name).map_err(fail)?.brief());
            }
        } else {
            let trimmed = body.trim();
            // The word a tag opens with, which need not be followed by a
            // space: profiles write `{elsif(a > b)}`.
            let word = trimmed
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(trimmed.len());
            let (keyword, argument) = match &trimmed[..word] {
                keyword @ ("if" | "elsif" | "elseif" | "else" | "endif") => {
                    (keyword, trimmed[word..].trim_start())
                }
                _ => trimmed
                    .split_once(char::is_whitespace)
                    .unwrap_or((trimmed, "")),
            };
            let script = expression::is_script(trimmed);
            match keyword {
                _ if script => {
                    if live {
                        out.push_str(&expression::run(trimmed, variables, overlay).map_err(fail)?);
                    }
                }
                "if" => {
                    let truth = live
                        && expression::evaluate(argument, variables, overlay)
                            .map_err(fail)?
                            .truthy()
                            .map_err(fail)?;
                    branches.push(Branch {
                        emitting: truth,
                        taken: truth,
                        outer: live,
                        line,
                    });
                }
                "elsif" | "elseif" | "else" => {
                    let branch = branches
                        .last_mut()
                        .ok_or_else(|| fail(format!("{{{keyword}}} without an {{if}}")))?;
                    let wanted = branch.outer && !branch.taken;
                    let truth = wanted
                        && (keyword == "else"
                            || expression::evaluate(argument, variables, overlay)
                                .map_err(fail)?
                                .truthy()
                                .map_err(fail)?);
                    branch.emitting = truth;
                    branch.taken |= truth;
                }
                "endif" => {
                    branches
                        .pop()
                        .ok_or_else(|| fail("{endif} without an {if}".into()))?;
                }
                _ if live => {
                    out.push_str(
                        &expression::evaluate(trimmed, variables, overlay)
                            .map_err(fail)?
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
        line += body.matches('\n').count();
    }
    if let Some(open) = branches.last() {
        return Err(TemplateError {
            line: open.line,
            message: "this {if} is never closed with {endif}".into(),
        });
    }
    out.push_str(rest);
    Ok(out)
}

/// Position of the bracket that closes the one `tag` opens with, skipping
/// brackets inside string literals and nested index brackets.
fn closing(tag: &str, close: char) -> Option<usize> {
    let open = tag.chars().next()?;
    let (mut depth, mut in_string) = (0usize, false);
    for (position, c) in tag.char_indices() {
        match c {
            '"' => in_string = !in_string,
            _ if in_string => {}
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(position);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables() -> Variables {
        let mut variables = Variables::default();
        variables.set("layer_z", Value::Float(0.25));
        variables.set("layer_num", Value::Int(4));
        variables.set("total_layer_count", Value::Int(389));
        variables.set("initial_extruder", Value::Int(0));
        variables.set("curr_bed_type", Value::text("Cool Plate"));
        variables.set("print_sequence", Value::text("by layer"));
        variables.set("travel_speed", Value::text("500"));
        variables.set_list(
            "nozzle_temperature",
            vec![Value::text("220"), Value::text("240")],
        );
        variables
    }

    fn expanded(template: &str) -> String {
        expand(template, &variables()).unwrap()
    }

    #[test]
    fn text_passes_through_and_placeholders_are_filled() {
        assert_eq!(expanded("G28 X Y\nM104 S140\n"), "G28 X Y\nM104 S140\n");
        assert_eq!(expanded(";[layer_z]\n"), ";0.25\n");
        assert_eq!(
            expanded(
                "SET_PRINT_STATS_INFO TOTAL_LAYER={total_layer_count} CURRENT_LAYER={layer_num+1}"
            ),
            "SET_PRINT_STATS_INFO TOTAL_LAYER=389 CURRENT_LAYER=5"
        );
        assert_eq!(
            expanded("M104 T{initial_extruder} S{nozzle_temperature[initial_extruder] - 90}"),
            "M104 T0 S130"
        );
        assert_eq!(expanded("M109 S[nozzle_temperature]"), "M109 S220");
    }

    #[test]
    fn branches_keep_only_the_text_that_applies() {
        let template = "{if curr_bed_type==\"High Temp Plate\"}  \nG28 Z Z_OFFSET -0.07\n{elsif curr_bed_type==\"Cool Plate\"} \nG28 Z COOL\n{else}\nG28 Z\n{endif} \nG90\n";
        // The directives vanish; whatever shared their lines stays.
        assert_eq!(expanded(template), " \nG28 Z COOL\n \nG90\n");

        let end = "{if print_sequence == \"by object\"}\nG91\n{endif}\nPRINT_END\n";
        assert_eq!(expanded(end), "\nPRINT_END\n");
        let nested =
            "{if layer_num > 2}a{if layer_num > 10}b{else}c{endif}d{else}e{if true}f{endif}{endif}";
        assert_eq!(expanded(nested), "acd");
    }

    #[test]
    fn untaken_branches_are_not_evaluated() {
        // The unknown variable sits in a branch that is skipped.
        assert_eq!(expanded("{if layer_num < 0}{no_such_thing}{endif}ok"), "ok");
        assert_eq!(expanded("{if false}[no_such_thing]{endif}ok"), "ok");
    }

    #[test]
    fn mistakes_name_their_line() {
        let error = |template: &str| expand(template, &variables()).unwrap_err();
        assert_eq!(
            error("G28\nM104 S{bed_temp}\n"),
            TemplateError {
                line: 2,
                message: "unknown variable \"bed_temp\"".into()
            }
        );
        assert_eq!(error("a\nb\n{if layer_num > 1}\nc").line, 3);
        assert!(error("{endif}").message.contains("without an {if}"));
        assert!(error("{else}").message.contains("without an {if}"));
        assert!(error("G1 X{layer_num").message.contains("never closed"));
        assert!(error("[layer z]").message.contains("not a variable name"));
        assert!(
            error("{layer_num[0]}")
                .message
                .contains("cannot be indexed")
        );
    }

    /// PlaceholderParser.cpp reads entry 0 of a list asked for an entry it
    /// does not have, which Bambu's filament change counts on.
    #[test]
    fn a_list_read_past_its_end_gives_its_first_entry() {
        assert_eq!(expanded("{nozzle_temperature[5]}"), "220");
        assert_eq!(expanded("{nozzle_temperature[0 - 1]}"), "220");
        assert_eq!(expanded("{nozzle_temperature[1]}"), "240");
        assert_eq!(
            expanded("{if nozzle_temperature[layer_num] > 230}a{else}b{endif}"),
            "b"
        );
    }

    #[test]
    fn scripts_declare_set_branch_and_write() {
        let script = "{\nlocal max_speed = 350.0;\nlocal speed = max_speed;\nif travel_speed < max_speed then\n  speed = travel_speed;\nendif\n\"G1 F\" + (speed * 60) + \"\n\";\n}";
        assert_eq!(expanded(script), "G1 F21000\n");
        // Branches not taken are passed over unread, nested ones included.
        let branches = "{if layer_num < 0 then no_such_thing; if a then b endif elsif layer_num == 4 then \"four\"; else nor_this; endif \"!\"}";
        assert_eq!(expanded(branches), "four!");
        let fallen = "{if false then 1 elsif false then 2 else \"last\" endif}";
        assert_eq!(expanded(fallen), "last");
        let empty = "{if true then \"a\"; else endif; if false then else \"b\" endif}";
        assert_eq!(expanded(empty), "ab");
        // Numbers are written as the reference's stream output writes them.
        assert_eq!(
            expanded("{local third = 1.0 / 3; third; \" \" + 2.50}"),
            "0.333333 2.5"
        );
        // A local lasts to the end of the template, across blocks.
        assert_eq!(expanded("{local n = 2;}{n * layer_num}[layer_num]"), "84");
    }

    #[test]
    fn what_a_script_sets_can_be_kept() {
        let mut variables = variables();
        variables.set_list(
            "position",
            vec![Value::Float(10.0), Value::Float(20.0), Value::Float(0.4)],
        );
        let script = "{position[2] = position[2] + 2.0; global seen = layer_num + 1; local gone = 9; layer_num = 7;}";
        // Discarded by default.
        assert_eq!(expand(script, &variables).unwrap(), "");
        assert_eq!(variables.value("layer_num"), Some(Value::Int(4)));
        // Kept on request: values of what was given, and globals, not locals.
        assert_eq!(expand_mut(script, &mut variables).unwrap(), "");
        assert_eq!(variables.list("position").unwrap()[2], Value::Float(2.4));
        assert_eq!(variables.value("seen"), Some(Value::Int(5)));
        assert_eq!(variables.value("layer_num"), Some(Value::Int(7)));
        assert!(!variables.contains("gone"));
    }

    #[test]
    fn script_mistakes_are_errors() {
        let error = |template: &str| expand(template, &variables()).unwrap_err().message;
        assert!(error("{undeclared = 1;}").contains("never declared"));
        assert!(error("{nozzle_temperature = 1;}").contains("is a list"));
        assert!(error("{nozzle_temperature[7] = 1;}").contains("no entry 7"));
        assert!(error("{layer_num[0] = 1;}").contains("cannot be indexed"));
        assert!(error("{if true then 1;}").contains("endif"));
        assert!(error("{local a = 1 local b = 2;}").contains("expected \";\""));
        assert!(error("{local a = (1, 2);}").contains("expected"));
    }

    #[test]
    fn a_condition_may_follow_its_keyword_without_a_space() {
        let mut variables = Variables::default();
        variables.set("bed", Value::Int(52));
        let template = "{if(bed > 55)}hot{elsif(bed > 50)||(bed < 0)}warm{elsif(bed > 45)}mild{else}cold{endif}";
        assert_eq!(expand(template, &variables).unwrap(), "warm");
        variables.set("bed", Value::Int(60));
        assert_eq!(expand(template, &variables).unwrap(), "hot");
        variables.set("bed", Value::Int(20));
        assert_eq!(expand(template, &variables).unwrap(), "cold");
    }
}
