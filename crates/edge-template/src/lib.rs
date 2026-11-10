//! The placeholder language of custom G-code: the machine's start, end and
//! layer-change snippets are templates, and this fills them in.
//!
//! Covered, after the reference's PlaceholderParser: `{expression}`,
//! `{if}` / `{elsif}` / `{else}` / `{endif}`, and the older `[variable]`
//! form. Expressions have numbers, strings, booleans, variables with an
//! optional `[index]`, arithmetic, comparison, `and` / `or` / `not`, a
//! ternary, and a few functions.
//!
//! Not covered: the statement form (`{ local x = 1; if x then ... endif }`)
//! that newer tool-change templates use, regular-expression matching, and
//! assignment. A template using them is an error, never a silent skip.

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
enum Variable {
    One(Value),
    /// One entry per extruder or filament.
    Many(Vec<Value>),
}

impl Variables {
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.values.insert(name.into(), Variable::One(value));
    }

    pub fn set_list(&mut self, name: impl Into<String>, values: Vec<Value>) {
        self.values.insert(name.into(), Variable::Many(values));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// A variable's value; `index` picks an entry of a list. A list read
    /// without an index gives its first entry, as the reference does for
    /// the older `[variable]` form.
    fn get(&self, name: &str, index: Option<i64>) -> Result<Value, String> {
        let variable = self
            .values
            .get(name)
            .ok_or_else(|| format!("unknown variable \"{name}\""))?;
        match (variable, index) {
            (Variable::One(value), None) => Ok(value.clone()),
            (Variable::One(_), Some(_)) => Err(format!(
                "\"{name}\" is a single value and cannot be indexed"
            )),
            (Variable::Many(values), index) => {
                let index = index.unwrap_or(0);
                usize::try_from(index)
                    .ok()
                    .and_then(|index| values.get(index))
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "\"{name}\" has {} entries; there is no entry {index}",
                            values.len()
                        )
                    })
            }
        }
    }
}

/// Fills in a template. Text outside braces and brackets is copied as it
/// is, including the rest of a line that held only a directive, which is
/// what the reference does too.
pub fn expand(template: &str, variables: &Variables) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
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
                out.push_str(&variables.get(name, None).map_err(fail)?.to_string());
            }
        } else {
            let trimmed = body.trim();
            let (keyword, argument) = trimmed
                .split_once(char::is_whitespace)
                .unwrap_or((trimmed, ""));
            match keyword {
                "if" => {
                    let truth = live
                        && expression::evaluate(argument, variables)
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
                            || expression::evaluate(argument, variables)
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
                        &expression::evaluate(trimmed, variables)
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
            error("{nozzle_temperature[5]}")
                .message
                .contains("no entry 5")
        );
        assert!(
            error("{layer_num[0]}")
                .message
                .contains("cannot be indexed")
        );
    }

    #[test]
    fn statement_blocks_are_refused_not_mangled() {
        let script = "{\nlocal max_speed = 350.0;\nif travel_speed < max_speed then\n\"G1 F\" + travel_speed;\nendif\n}";
        assert!(expand(script, &variables()).is_err());
    }
}
