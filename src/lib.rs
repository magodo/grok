//! The `grok` library allows you to quickly parse and match potentially unstructured data
//! into a structed result. It is especially helpful when parsing logfiles of all kinds. This
//! [Rust](http://rust-lang.org) version is mainly a port from the [java version](https://github.com/thekrakken/java-grok)
//! which in drew inspiration from the original [ruby version](https://github.com/logstash-plugins/logstash-filter-grok).
#![doc(html_root_url = "https://docs.rs/grok/0.1.0")]
extern crate regex;

use regex::{Captures, Regex};
use std::collections::BTreeMap;
use std::fmt;
use std::error::Error as StdError;

const MAX_RECURSION: usize = 1024;
const GROK_PATTERN: &'static str = r"%\{(?P<name>(?P<pattern>[A-z0-9]+)(?::(?P<alias>[A-z0-9_:;/\s\.]+))?)(?:=(?P<definition>(?:(?:[^{}]+|\.+)+)+))?\}";

/// The `Matches` represent matched results from a `Pattern` against text.
#[derive(Debug)]
pub struct Matches<'a> {
    captures: Captures<'a>,
    alias: &'a BTreeMap<String, String>
}

impl<'a> Matches<'a> {
    /// Instantiates the matches for a pattern after the match.
    pub fn new(captures: Captures<'a>, alias: &'a BTreeMap<String, String>) -> Self {
        Matches { captures: captures, alias: alias }
    }

    /// Gets the value for the name (or) alias if found, `None` otherwise.
    pub fn get(&self, name_or_alias: &str) -> Option<&str> {
        match self.alias.get(name_or_alias) {
            Some(real) => self.captures.name(real).map(|m| m.as_str()),
            None => None,
        }
    }

    /// Returns the number of matches.
    pub fn len(&self) -> usize {
        self.captures.len() - 1
    }

    /// Returns true if there are no matches, false otherwise.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
} 

/// The `Pattern` represents a compiled regex, ready to be matched against arbitrary text.
#[derive(Debug)]
pub struct Pattern {
    regex: Regex,
    alias: BTreeMap<String, String>,
}

impl Pattern {
    /// Creates a new pattern from a raw regex string and an alias map to identify the
    /// fields properly.
    pub fn new(regex: &str, alias: BTreeMap<String, String>) -> Result<Self, Error> {
        match Regex::new(regex) {
            Ok(r) => Ok (Pattern { regex: r, alias: alias }),
            Err(_) => Err(Error::RegexCompilationFailed(regex.into())),
        }
    }

    /// Matches this compiled `Pattern` against the text and returns the matches.
    pub fn match_against<'a>(&'a self, text: &'a str) -> Option<Matches<'a>> {
        self.regex.captures(text).map(|cap| Matches::new(cap, &self.alias))
    }
}

/// The basic structure to manage patterns, entry point for common usage.
#[derive(Debug)]
pub struct Grok {
    definitions: BTreeMap<String, String>,
}

impl Grok {
    /// Creates a new `Grok` instance with no patterns.
    pub fn empty() -> Self {
        Grok {
            definitions: BTreeMap::new(),
        }
    }

    /// Inserts a custom pattern.
    pub fn insert_definition<S: Into<String>>(&mut self, name: S, pattern: S) {
        self.definitions.insert(name.into(), pattern.into());
    }

    /// Compiles the given pattern, making it ready for matching.
    pub fn compile(&mut self, pattern: &str, with_alias_only: bool) -> Result<Pattern, Error> {
        let mut named_regex = String::from(pattern);
        let mut alias: BTreeMap<String, String> = BTreeMap::new();

        let mut index = 0;
        let mut iteration_left = MAX_RECURSION;
        let mut continue_iteration = true;

        let grok_regex = match Regex::new(GROK_PATTERN) {
            Ok(r) => r,
            Err(_) => return Err(Error::RegexCompilationFailed(GROK_PATTERN.into())),
        };

        while continue_iteration {
            continue_iteration = false;
            if iteration_left <= 0 {
                return Err(Error::RecursionTooDeep);
            }
            iteration_left -= 1;

            if let Some(m) = grok_regex.captures(&named_regex.clone()) {
                continue_iteration = true;
                let raw_pattern = match m.name("pattern") {
                    Some(p) => p.as_str(),
                    None => return Err(Error::GenericCompilationFailure("Could not find pattern in matches".into())),
                };

                let mut name = match m.name("name") {
                    Some(n) => String::from(n.as_str()),
                    None => return Err(Error::GenericCompilationFailure("Could not find name in matches".into())),
                };
                if let Some(definition) = m.name("definition") {
                    self.insert_definition(raw_pattern, definition.as_str());
                    name = format!("{}={}", name, definition.as_str());
                }

                // Since a pattern with a given name can show up more than once, we need to
                // loop through the number of matches found and apply the transformations
                // on each of them.
                for _ in 0..named_regex.matches(&format!("%{{{}}}", name)).count() {

                    // Check if we have a definition for the raw pattern key and fail quickly
                    // if not.
                    let pattern_definition = match self.definitions.get(raw_pattern) {
                        Some(d) => d,
                        None => return Err(Error::DefinitionNotFound(raw_pattern.into())),
                    };

                    // If no alias is specified and all but with alias are ignored, the replacement
                    // tells the regex engine to ignore the matches. Otherwise, the definition is
                    // turned into a regex that the engine understands and uses a named group.
                    let replacement = if with_alias_only && m.name("alias").is_none() {
                        format!("(?:{})", pattern_definition)
                    } else {
                        format!("(?P<name{}>{})", index, pattern_definition)
                    };

                    // If an alias is specified by the user use that one to match the name<index>
                    // conversion, oterhwise just use the name of the pattern definition directly.
                    alias.insert(match m.name("alias") {
                        Some(a) => a.as_str().into(),
                        None => name.clone(),
                    }, format!("name{}", index));


                    // Finally, look for the original %{...} style pattern and replace it
                    // with our replacement (only the first occurence since we are iterating
                    // one by one).
                    named_regex = named_regex.replacen(&format!("%{{{}}}", name), &replacement, 1);

                    index += 1;
                }
            }
        }


        if named_regex.is_empty() {
            Err(Error::CompiledPatternIsEmpty(pattern.into()))
        } else {
            Pattern::new(&named_regex, alias)
        }
    }
}

impl Default for Grok {
    fn default() -> Grok {
        Grok::empty()
    }
}

/// An error that occurred when using this library.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The recursion while compiling has exhausted the limit.
    RecursionTooDeep,
    /// After compiling, the resulting compiled regex pattern is empty.
    CompiledPatternIsEmpty(String),
    /// A corresponding pattern definition could not be found for the given name.
    DefinitionNotFound(String),
    /// If the compilation for a specific regex in the underlying engine failed.
    RegexCompilationFailed(String),
    /// Something is messed up during the compilation phase.
    GenericCompilationFailure(String),
    /// Hints that destructuring should not be exhaustive.
    ///
    /// This enum may grow additional variants, so this makes sure clients
    /// don't count on exhaustive matching. (Otherwise, adding a new variant
    /// could break existing code.)
    #[doc(hidden)]
    __Nonexhaustive,
}

impl StdError for Error {
    fn description(&self) -> &str {
        match *self {
            Error::RecursionTooDeep => "compilation recursion reached the limit",
            Error::CompiledPatternIsEmpty(_) => "compiled pattern is empty",
            Error::DefinitionNotFound(_) => "pattern definition not found while compiling",
            Error::RegexCompilationFailed(_) => "regex compilation in the engine failed",
            Error::GenericCompilationFailure(_) => "something happened during the compilation phase",
            Error::__Nonexhaustive => unreachable!(),
        }
    }

    fn cause(&self) -> Option<&StdError> {
        None
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::RecursionTooDeep => 
                write!(f, "Recursion while compiling reached the limit of {}", MAX_RECURSION),
            Error::CompiledPatternIsEmpty(ref p) => 
                write!(f, "The given pattern \"{}\" ended up compiling into an empty regex", p),
            Error::DefinitionNotFound(ref d) => 
                write!(f, "The given pattern definition name \"{}\" could not be found in the definition map", d),
            Error::RegexCompilationFailed(ref r) => 
                write!(f, "The given regex \"{}\" failed compilation in the underlying engine", r),
            Error::GenericCompilationFailure(ref d) =>
                write!(f, "Something unexpected happened during the compilation phase: \"{}\"", d),
            Error::__Nonexhaustive => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_simple_anonymous_pattern() {
        let mut grok = Grok::default();
        grok.insert_definition("USERNAME", r"[a-zA-Z0-9._-]+");
        let pattern = grok.compile("%{USERNAME}", false).expect("Error while compiling!");

        let matches = pattern.match_against("root").expect("No matches found!");
        assert_eq!("root", matches.get("USERNAME").unwrap());
        assert_eq!(1, matches.len());
        let matches = pattern.match_against("john doe").expect("No matches found!");
        assert_eq!("john", matches.get("USERNAME").unwrap());
        assert_eq!(1, matches.len());
    }

    #[test]
    fn test_simple_named_pattern() {
        let mut grok = Grok::default();
        grok.insert_definition("USERNAME", r"[a-zA-Z0-9._-]+");
        let pattern = grok.compile("%{USERNAME:usr}", false).expect("Error while compiling!");

        let matches = pattern.match_against("root").expect("No matches found!");
        assert_eq!("root", matches.get("usr").unwrap());
        assert_eq!(1, matches.len());
        let matches = pattern.match_against("john doe").expect("No matches found!");
        assert_eq!("john", matches.get("usr").unwrap());
        assert_eq!(1, matches.len());
    }

    #[test]
    fn test_alias_anonymous_pattern() {
        let mut grok = Grok::default();
        grok.insert_definition("USERNAME", r"[a-zA-Z0-9._-]+");
        grok.insert_definition("USER", r"%{USERNAME}");
        let pattern = grok.compile("%{USER}", false).expect("Error while compiling!");

        let matches = pattern.match_against("root").expect("No matches found!");
        assert_eq!("root", matches.get("USER").unwrap());
        let matches = pattern.match_against("john doe").expect("No matches found!");
        assert_eq!("john", matches.get("USER").unwrap());
    }

    #[test]
    fn test_ailas_named_pattern() {
        let mut grok = Grok::default();
        grok.insert_definition("USERNAME", r"[a-zA-Z0-9._-]+");
        grok.insert_definition("USER", r"%{USERNAME}");
        let pattern = grok.compile("%{USER:usr}", false).expect("Error while compiling!");

        let matches = pattern.match_against("root").expect("No matches found!");
        assert_eq!("root", matches.get("usr").unwrap());
        let matches = pattern.match_against("john doe").expect("No matches found!");
        assert_eq!("john", matches.get("usr").unwrap());
    }

    #[test]
    fn test_composite_or_pattern() {
        let mut grok = Grok::default();
        grok.insert_definition("MAC", r"(?:%{CISCOMAC}|%{WINDOWSMAC}|%{COMMONMAC})");
        grok.insert_definition("CISCOMAC", r"(?:(?:[A-Fa-f0-9]{4}\.){2}[A-Fa-f0-9]{4})");
        grok.insert_definition("WINDOWSMAC", r"(?:(?:[A-Fa-f0-9]{2}-){5}[A-Fa-f0-9]{2})");
        grok.insert_definition("COMMONMAC", r"(?:(?:[A-Fa-f0-9]{2}:){5}[A-Fa-f0-9]{2})");
        let pattern = grok.compile("%{MAC}", false).expect("Error while compiling!");

        let matches = pattern.match_against("5E:FF:56:A2:AF:15").expect("No matches found!");
        assert_eq!("5E:FF:56:A2:AF:15", matches.get("MAC").unwrap());
        assert_eq!(4, matches.len());
        let matches = pattern.match_against("hello! 5E:FF:56:A2:AF:15 what?").expect("No matches found!");
        assert_eq!("5E:FF:56:A2:AF:15", matches.get("MAC").unwrap());
        assert_eq!(true, pattern.match_against("5E:FF").is_none());
    }

    #[test]
    fn test_multiple_patterns() {
        let mut grok = Grok::default();
        grok.insert_definition("YEAR", r"(\d\d){1,2}");
        grok.insert_definition("MONTH", r"\b(?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|Jun(?:e)?|Jul(?:y)?|Aug(?:ust)?|Sep(?:tember)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\b");
        grok.insert_definition("DAY", r"(?:Mon(?:day)?|Tue(?:sday)?|Wed(?:nesday)?|Thu(?:rsday)?|Fri(?:day)?|Sat(?:urday)?|Sun(?:day)?)");
        let pattern = grok.compile("%{DAY} %{MONTH} %{YEAR}", false).expect("Error while compiling!");

        let matches = pattern.match_against("Monday March 2012").expect("No matches found!");
        assert_eq!("Monday", matches.get("DAY").unwrap());
        assert_eq!("March", matches.get("MONTH").unwrap());
        assert_eq!("2012", matches.get("YEAR").unwrap());
        assert_eq!(None, matches.get("unknown"));
    }

    #[test]
    fn test_with_alias_only() {
        let mut grok = Grok::default();
        grok.insert_definition("MAC", r"(?:%{CISCOMAC}|%{WINDOWSMAC}|%{COMMONMAC})");
        grok.insert_definition("CISCOMAC", r"(?:(?:[A-Fa-f0-9]{4}\.){2}[A-Fa-f0-9]{4})");
        grok.insert_definition("WINDOWSMAC", r"(?:(?:[A-Fa-f0-9]{2}-){5}[A-Fa-f0-9]{2})");
        grok.insert_definition("COMMONMAC", r"(?:(?:[A-Fa-f0-9]{2}:){5}[A-Fa-f0-9]{2})");
        let pattern = grok.compile("%{MAC:macaddr}", true).expect("Error while compiling!");

        let matches = pattern.match_against("5E:FF:56:A2:AF:15").expect("No matches found!");
        assert_eq!("5E:FF:56:A2:AF:15", matches.get("macaddr").unwrap());
        assert_eq!(1, matches.len());
        let matches = pattern.match_against("hello! 5E:FF:56:A2:AF:15 what?").expect("No matches found!");
        assert_eq!("5E:FF:56:A2:AF:15", matches.get("macaddr").unwrap());
        assert_eq!(true, pattern.match_against("5E:FF").is_none());
    }
}
