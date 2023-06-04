//! # Test Utilities
//!
//! For now, mostly just helpers for running "ui tests", or executing forth code at
//! test time.
//!
//! ## UI Tests
//!
//! Generally, forth code provided as a str will have one of the following things
//! for each line:
//!
//! * Configuration values for the VM, specified as "frontmatter comments".
//!   These must appear before any other non-comment lines. Currently accepted:
//!     * `( data_stack_elems USIZE )`
//!     * `( return_stack_elems USIZE )`
//!     * `( control_stack_elems USIZE )`
//!     * `( input_buf_elems USIZE )`
//!     * `( output_buf_elems USIZE )`
//!     * `( dict_buf_elems USIZE )`
//! * Comment lines. These are any lines just containing a `( ... )` style forth comment.
//! * Successful input lines, starting with `> ...`.
//! * Successful output lines, starting with `< ...`.
//!     * Any successful input line can have zero or more output lines
//!     * If *no* input lines are specified, ANY successful output is accepted/ignored.
//! * Unsuccessful input lines, starting with `x ...`.
//!     * This line is expected to cause an "exception" - basically `process_line` returns
//!       an `Err()`.
//!     * There is no way to specify which error yet
//!     * Unsuccessful input lines may not have any successful output
//!
//! These ui-tests can also be run as doctests (see below), and doctests can be run
//! in miri.
//!
//! ### Example
//!
//! This is a forth ui-test doctest. It will be run with `cargo test --all-features`.
//!
//! ```rust
//! # use forth3::testutil::blocking_runtest;
//! #
//! # blocking_runtest(r#"
//! ( specify VM settings with frontmatter )
//! ( data_stack_elems 1 )
//!
//! ( specify input with no output )
//! > : star 42 emit ;
//!
//! ( specify input and output )
//! > star
//! < *ok.
//!
//! ( specify lines that cause exceptions/errors )
//! x starb
//! # "#)
//! ```

use crate::{leakbox::{LBForthParams, LBForth}, Forth, Error};

/// Run the given forth ui test against ALL enabled forth VMs
///
/// A helper for calling blocking + async runtest functions. This is a good
/// default to use for unit tests.
pub fn all_runtest(contents: &str) {
    blocking_runtest(contents);
    #[cfg(feature = "async")]
    async_blockon_runtest(contents);
}

/// Run the given forth ui test against the default forth vm
///
/// Does accept any/all/none of the following configuration frontmatter (see above
/// for listing of frontmatter kinds)
pub fn blocking_runtest(contents: &str) {
    let tokd = tokenize(contents, true).unwrap();
    let mut forth = LBForth::from_params(tokd.settings, (), Forth::FULL_BUILTINS);
    blocking_steps_with(tokd.steps.as_slice(), &mut forth.forth);
}

/// Run the given forth ui-test against the given forth vm.
///
/// Does not accept ui-tests with frontmatter configuration (will panic)
pub fn blocking_runtest_with<T>(forth: &mut Forth<T>, contents: &str) {
    let tokd = tokenize(contents, false).unwrap();
    blocking_steps_with(tokd.steps.as_slice(), forth);
}

/// Run the given forth ui test against the async forth vm
///
/// Does accept any/all/none of the following configuration frontmatter (see above
/// for listing of frontmatter kinds). Provides no actual async builtins, and will
/// panic if the provided dispatcher is called for some reason
#[cfg(feature = "async")]
pub fn async_blockon_runtest(contents: &str)
{
    use crate::{leakbox::AsyncLBForth, dictionary::{AsyncBuiltinEntry, AsyncBuiltins}, fastr::FaStr};

    struct TestAsyncDispatcher;
    impl<'forth> AsyncBuiltins<'forth, ()> for TestAsyncDispatcher {
        type Future = futures::future::Ready<Result<(), Error>>;
        const BUILTINS: &'static [AsyncBuiltinEntry<()>] = &[];
        fn dispatch_async(
            &self,
            _id: &FaStr,
            _forth: &'forth mut Forth<()>,
        ) -> Self::Future {
             unreachable!("no async builtins should be called in this test")
        }
    }

    let tokd = tokenize(contents, true).unwrap();
    let mut forth = AsyncLBForth::from_params(tokd.settings, (), Forth::FULL_BUILTINS, TestAsyncDispatcher);
    async_blockon_runtest_with(&mut forth.forth, contents);
}

/// Like `async_blockon_runtest`, but with provided context + dispatcher
#[cfg(feature = "async")]
pub fn async_blockon_runtest_with_dispatcher<T, D>(context: T, dispatcher: D, contents: &str)
where
    T: 'static,
    D: for<'forth> crate::dictionary::AsyncBuiltins<'forth, T>,
{
    use crate::leakbox::AsyncLBForth;

    let tokd = tokenize(contents, true).unwrap();
    let mut forth = AsyncLBForth::from_params(tokd.settings, context, Forth::FULL_BUILTINS, dispatcher);
    async_blockon_runtest_with(&mut forth.forth, contents);
}

/// Like `async_blockon_runtest`, but with provided async vm
#[cfg(feature = "async")]
pub fn async_blockon_runtest_with<T, D>(forth: &mut crate::AsyncForth<T, D>, contents: &str)
where
    T: 'static,
    D: for<'forth> crate::dictionary::AsyncBuiltins<'forth, T>,
{
    let tokd = tokenize(contents, false).unwrap();
    for Step { ref input, output: ref outcome } in tokd.steps {
        #[cfg(not(miri))]
        println!("> {input}");
        forth.input_mut().fill(input).unwrap();
        let res = futures::executor::block_on(forth.process_line());
        check_output(res, outcome, forth.output().as_str());
        forth.output_mut().clear();
    }
}

fn check_output(res: Result<(), Error>, outcome: &Outcome, output: &str) {
    #[cfg(not(miri))]
    println!("< {output}");
    match (res, outcome) {
        (Ok(()), Outcome::OkAnyOutput) => {}
        (Ok(()), Outcome::OkWithOutput(exp)) => {
            let act_lines = output.lines().collect::<Vec<&str>>();
            assert_eq!(act_lines.len(), exp.len());
            act_lines.iter().zip(exp.iter()).for_each(|(a, e)| {
                assert_eq!(a.trim_end(), e.trim_end());
            })
        }
        (Err(_e), Outcome::FatalError) => {}
        (res, exp) => {
            eprintln!("Error!");
            eprintln!("Expected: {exp:?}");
            eprintln!("Got: {res:?}");
            if res.is_ok() {
                eprintln!("Output:\n{}", output);
            }
            panic!();
        }
    }
}

// Runs the given steps against the given forth VM.
//
// Panics on any mismatch
fn blocking_steps_with<T>(steps: &[Step], forth: &mut Forth<T>) {
    for Step { input, output: outcome } in steps {
        #[cfg(not(miri))]
        println!("> {input}");
        forth.input.fill(input).unwrap();
        let res = forth.process_line();
        check_output(res, outcome, forth.output.as_str());
        forth.output.clear();
    }
}

#[derive(Debug)]
enum Outcome {
    OkAnyOutput,
    OkWithOutput(Vec<String>),
    FatalError,
}

#[derive(Debug)]
struct Step {
    input: String,
    output: Outcome,
}

#[derive(Default, Debug)]
struct Tokenized {
    settings: LBForthParams,
    steps: Vec<Step>,
}

fn tokenize(contents: &str, allow_frontmatter: bool) -> Result<Tokenized, ()> {
    let mut lines = contents.lines();
    let mut output = Tokenized::default();
    let mut frontmatter_done = !allow_frontmatter;

    while let Some(line) = lines.next() {
        let (tok, remain) = if let Some(t) = line.trim_start().split_once(" ") {
            t
        } else {
            continue;
        };

        match tok {
            ">" => {
                frontmatter_done = true;
                output.steps.push(Step {
                    input: remain.to_string(),
                    output: Outcome::OkAnyOutput,
                });
            }
            "<" => {
                frontmatter_done = true;
                let cur_step = output.steps.last_mut().unwrap();
                let expected_out = remain.to_string();
                match &mut cur_step.output {
                    Outcome::OkAnyOutput => {
                        cur_step.output = Outcome::OkWithOutput(vec![expected_out]);
                    },
                    Outcome::OkWithOutput(o) => {
                        o.push(remain.to_string());
                    },
                    Outcome::FatalError => panic!("Fatal error can't set output"),
                }
            }
            "x" => {
                frontmatter_done = true;
                output.steps.push(Step {
                    input: remain.to_string(),
                    output: Outcome::FatalError,
                });
            }
            "(" => {
                let mut split = remain.split_whitespace();
                let mut is_comment = false;
                match split.next() {
                    Some("data_stack_elems") => {
                        output.settings.data_stack_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some("return_stack_elems") => {
                        output.settings.return_stack_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some("control_stack_elems") => {
                        output.settings.control_stack_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some("input_buf_elems") => {
                        output.settings.input_buf_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some("output_buf_elems") => {
                        output.settings.output_buf_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some("dict_buf_elems") => {
                        output.settings.dict_buf_elems = split.next().unwrap().parse::<usize>().unwrap();
                    }
                    Some(_) => {
                        is_comment = true;
                    }
                    _ => panic!(),
                }
                if !is_comment {
                    assert!(!frontmatter_done, "Unexpected frontmatter settings!");
                    assert_eq!(Some(")"), split.next());
                }
            }
            _ => {}
        }
    }

    Ok(output)
}
