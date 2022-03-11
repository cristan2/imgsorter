use std::collections::hash_set::Iter;
use std::collections::HashSet;
use std::iter::Cloned;

pub struct ColoredString;

/// Provides static methods for formatting colored text based on ANSI codes
/// Taken from the following SO answers:
/// * [https://stackoverflow.com/questions/69981449/how-do-i-print-colored-text-to-the-terminal-in-rust]
/// * [https://stackoverflow.com/questions/287871/how-to-print-colored-text-to-the-terminal/287944#287944]
impl ColoredString {

    // Color codes:
    // * MAGENTA   = '\x1b[95m'
    // * BLUE      = '\x1b[94m'
    // * CYAN      = '\x1b[96m'
    // * GREEN     = '\x1b[92m'
    // * ORANGE    = '\x1b[93m'
    // * RED       = '\x1b[91m'
    // * NO_COLOR  = '\x1b[0m'
    // * BOLD      = '\x1b[1m'
    // * UNDERLINE = '\x1b[4m'

    pub fn magenta(s: &str) -> String { format!("\x1b[95m{}\x1b[0m", s) }
    pub fn blue(s: &str) -> String { format!("\x1b[94m{}\x1b[0m", s) }
    pub fn cyan(s: &str) -> String { format!("\x1b[96m{}\x1b[0m", s) }
    pub fn green(s: &str) -> String { format!("\x1b[92m{}\x1b[0m", s) }
    pub fn red(s: &str) -> String { format!("\x1b[91m{}\x1b[0m", s) }
    pub fn no_color(s: &str) -> String { format!("\x1b[0m{}\x1b[0m", s) }
    pub fn orange(s: &str) -> String { format!("\x1b[93m{}\x1b[0m", s) }
    pub fn bold_white(s: &str) -> String { format!("\x1b[1m{}\x1b[0m", s) }
    pub fn underline(s: &str) -> String { format!("\x1b[4m{}\x1b[0m", s) }

    pub fn warn_arrow() -> String { Self::orange(">") }
}

pub enum OutputColor {
    Error,
    Warning,
    Neutral,
    Good
}

pub struct RightPadding;

impl RightPadding {
    // TODO 5g - have char as argument
    pub fn space(str: String, pad_width: usize) -> String {
        format!("{:<width$}", str, width=pad_width)
    }

    pub fn dot(str: String, pad_width: usize) -> String {
        format!("{:.<width$}", str, width=pad_width)
    }

    pub fn dash(str: String, pad_width: usize) -> String {
        format!("{:-<width$}", str, width=pad_width)
    }

    pub fn em_dash(str: String, pad_width: usize) -> String {
        format!("{:─<width$}", str, width=pad_width)
    }

    pub fn middle_dot(str: String, pad_width: usize) -> String {
        format!("{:·<width$}", str, width=pad_width)
    }
}

pub const SEPARATOR_STATUS: &'static str = "...";
pub const SEPARATOR_DRY_RUN: &'static str = "--->";
pub const SEPARATOR_COPY_MOVE: &'static str = "──>";
pub const FILE_TREE_ENTRY: &'static str = " └── ";
pub const FILE_TREE_INDENT: &'static str = " |   ";

pub fn indent_string(indent_level: usize, file_name: String) -> String {
    let indents = FILE_TREE_INDENT.repeat(indent_level);
    format!("{}{}{}", indents, FILE_TREE_ENTRY.to_string(), file_name)
}


pub fn check_sets() {
    let a: HashSet<_> = ["unu", "doi", "trei"].iter().cloned().map(|s|String::from(s)).collect();
    let b: HashSet<_> = ["trei", "patru", "cinci"].iter().cloned().map(|s|String::from(s)).collect();
    let c: HashSet<_> = ["unu", "trei", "patru", "cinci", "sase"].iter().cloned().map(|s|String::from(s)).collect();
    let d: HashSet<_> = ["unu", "sase", "sapte"].iter().cloned().map(|s|String::from(s)).collect();
    let e: HashSet<_> = ["unu", "sase", "sapte"].iter().cloned().map(|s|String::from(s)).collect();

    let dirs: Vec<HashSet<String>> = vec![a, b, c, d, e];

    // let diff_ref: &HashSet<&String> = &a.difference(&b).collect();

    // let s: HashSet<&String> = b.difference(&a).collect();
    // let d: HashSet<&String> = c.difference(&a).collect();

    dirs.iter().enumerate().for_each( |(ix, set)| println!("{:?} -> {:?}", ix, set));

    fn difference_to_index(current_index: usize, all_dirs: &Vec<HashSet<String>>) -> HashSet<String> {
        let start_dir = all_dirs[current_index].clone();
        let slice = &all_dirs[0..current_index];
        slice.iter()
            .fold(start_dir, |acc: HashSet<String>, current_dir| {
                // println!("accum = {:?}, \ncurr_dir = {:?}", &acc, &current_dir);

                let diff: HashSet<String> = acc
                    .difference(current_dir)
                    .map(|d| d.clone())
                    .collect::<HashSet<_>>();

                diff
            })
    }

    let reduced_sets = dirs.iter().enumerate()
        .map( |(curr_ix, _) |
            difference_to_index(curr_ix, &dirs) )
        .collect::<Vec<_>>();

    println!("==================");
    // dbg!(reduced_sets);

    reduced_sets.iter().enumerate().for_each( |(ix, set)| println!("{:?} -> {:?}", ix, set))
}