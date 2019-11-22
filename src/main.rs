use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use serde::{Deserialize, Serialize};

/// If this is `true` then the output file we generate will not emit any
/// unsafe code. I'm not aware of any bugs with the unsafe code that I use and
/// thus this is by default set to `false`. Feel free to set it to `true` if
/// you are concerned.
const SAFE_ONLY: bool = false;

/// Representation of a grammar file in a Rust structure. This allows us to
/// use Serde to serialize and deserialize the json grammar files
#[derive(Serialize, Deserialize, Default, Debug)]
struct Grammar(BTreeMap<String, Vec<Vec<String>>>);

/// A strongly typed wrapper around a `usize` which selects different fragment
/// identifiers
#[derive(Clone, Copy, Debug)]
struct FragmentId(usize);

/// A fragment which is specified by the grammar file
#[derive(Clone, Debug)]
enum Fragment {
    /// A non-terminal fragment which refers to a list of `FragmentId`s to
    /// randomly select from for expansion
    NonTerminal(Vec<FragmentId>),

    /// A list of `FragmentId`s that should be expanded in order
    Expression(Vec<FragmentId>),

    /// A terminal fragment which simply should expand directly to the
    /// contained vector of bytes
    Terminal(Vec<u8>),

    /// A fragment which does nothing. This is used during optimization passes
    /// to remove fragments with no effect.
    Nop,
}

/// A grammar representation in Rust that is designed to be easy to work with
/// in-memory and optimized for code generation.
#[derive(Debug, Default)]
struct GrammarRust {
    /// All types
    fragments: Vec<Fragment>,

    /// Cached fragment identifier for the start node
    start: Option<FragmentId>,

    /// Mapping of non-terminal names to fragment identifers
    name_to_fragment: BTreeMap<String, FragmentId>,
}

impl GrammarRust {
    /// Create a new Rust version of a `Grammar` which was loaded via a
    /// grammar json specification.
    fn new(grammar: &Grammar) -> Self {
        // Create a new grammar structure
        let mut ret = GrammarRust::default();

        // Parse the input grammar to resolve all fragment names
        for (non_term, _) in grammar.0.iter() {
            // Make sure that there aren't duplicates of fragment names
            assert!(!ret.name_to_fragment.contains_key(non_term),
                "Duplicate non-terminal definition, fail");

            // Create a new, empty fragment
            let fragment_id = ret.allocate_fragment(
                Fragment::NonTerminal(Vec::new()));

            // Add the name resolution for the fragment
            ret.name_to_fragment.insert(non_term.clone(), fragment_id);
        }

        // Parse the input grammar
        for (non_term, fragments) in grammar.0.iter() {
            // Get the non-terminal fragment identifier
            let fragment_id = ret.name_to_fragment[non_term];

            // Create a vector to hold all of the variants possible under this
            // non-terminal fragment
            let mut variants = Vec::new();

            // Go through all sub-fragments
            for js_sub_fragment in fragments {
                // Different options for this sub-fragment
                let mut options = Vec::new();

                // Go through each option in the sub-fragment
                for option in js_sub_fragment {
                    let fragment_id = if let Some(&non_terminal) =
                            ret.name_to_fragment.get(option) {
                        // If we can resolve the name of this fragment, it is a
                        // non-terminal fragment and should be allocated as
                        // such
                        ret.allocate_fragment(
                            Fragment::NonTerminal(vec![non_terminal]))
                    } else {
                        // Convert the terminal bytes into a vector and
                        // create a new fragment containing it
                        ret.allocate_fragment(Fragment::Terminal(
                            option.as_bytes().to_vec()))
                    };

                    // Push this fragment as an option
                    options.push(fragment_id);
                }

                // Create a new fragment of all the options
                variants.push(
                    ret.allocate_fragment(Fragment::Expression(options)));
            }

            // Get access to the fragment we want to update based on the
            // possible variants
            let fragment = &mut ret.fragments[fragment_id.0];

            // Overwrite the terminal definition
            *fragment = Fragment::NonTerminal(variants);
        }

        // Resolve the start node
        ret.start = Some(ret.name_to_fragment["<start>"]);

        ret
    }

    /// Allocate a new fragment identifier and add it to the fragment list
    pub fn allocate_fragment(&mut self, fragment: Fragment) -> FragmentId {
        // Get a unique fragment identifier
        let fragment_id = FragmentId(self.fragments.len());

        // Store the fragment
        self.fragments.push(fragment);

        fragment_id
    }

    /// Optimize to remove fragments with non-random effects
    pub fn optimize(&mut self) {
        // Keeps track of fragment identifiers which resolve to nops
        let mut nop_fragments = BTreeSet::new();

        // Track if a optimization had an effect
        let mut changed = true;
        while changed {
            // Start off assuming no effect from optimzation
            changed = false;

            // Go through each fragment, looking for potential optimizations
            for idx in 0..self.fragments.len() {
                // Clone the fragment such that we can inspect it, but we also
                // can mutate it in place.
                match self.fragments[idx].clone() {
                    Fragment::NonTerminal(options) => {
                        // If this non-terminal only has one option, replace
                        // itself with the only option it resolves to
                        if options.len() == 1 {
                            self.fragments[idx] =
                                self.fragments[options[0].0].clone();
                            changed = true;
                        }
                    }
                    Fragment::Expression(expr) => {
                        // If this expression doesn't have anything to do at
                        // all. Then simply replace it with a `Nop`
                        if expr.len() == 0 {
                            self.fragments[idx] = Fragment::Nop;
                            changed = true;

                            // Track that this fragment identifier now resolves
                            // to a nop
                            nop_fragments.insert(idx);
                        }

                        // If this expression only does one thing, then replace
                        // the expression with the thing that it does.
                        if expr.len() == 1 {
                            self.fragments[idx] =
                                self.fragments[expr[0].0].clone();
                            changed = true;
                        }

                        // Remove all `Nop`s from this expression, as they
                        // wouldn't result in anything occuring.
                        if let Fragment::Expression(exprs) =
                                &mut self.fragments[idx] {
                            // Only retain fragments which are not nops
                            exprs.retain(|x| {
                                if nop_fragments.contains(&x.0) {
                                    // Fragment was a nop, remove it
                                    changed = true;
                                    false
                                } else {
                                    // Fragment was fine, keep it
                                    true
                                }
                            });
                        }
                    }
                    Fragment::Terminal(_) | Fragment::Nop => {
                        // Already maximally optimized
                    }
                }
            }
        }
    }

    /// Generate a new Rust program that can be built and will generate random
    /// inputs and benchmark them
    pub fn program<P: AsRef<Path>>(&self, path: P, max_depth: usize) {
        let mut program = String::new();

        // Construct the base of the application. This is a profiling loop that
        // is used for testing.
        program += &format!(r#"
#![allow(unused)]
use std::cell::Cell;
use std::time::Instant;

fn main() {{
    let mut fuzzer = Fuzzer {{
        seed:  Cell::new(0x34cc028e11b4f89c),
        buf:   Vec::new(),
    }};
    
    let mut generated = 0usize;
    let it = Instant::now();

    for iters in 1u64.. {{
        fuzzer.buf.clear();
        fuzzer.fragment_{}(0);
        generated += fuzzer.buf.len();

        // Filter to reduce the amount of times printing occurs
        if (iters & 0xfffff) == 0 {{
            let elapsed = (Instant::now() - it).as_secs_f64();
            let bytes_per_sec = generated as f64 / elapsed;
            print!("MiB/sec: {{:12.4}}\n", bytes_per_sec / 1024. / 1024.);
        }}
    }}
}}

struct Fuzzer {{
    seed:  Cell<usize>,
    buf:   Vec<u8>,
}}

impl Fuzzer {{
    fn rand(&self) -> usize {{
        let mut seed = self.seed.get();
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 43;
        self.seed.set(seed);
        seed
    }}
"#, self.start.unwrap().0);

        // Go through each fragment in the list of fragments
        for (id, fragment) in self.fragments.iter().enumerate() {
            // Create a new function for this fragment
            program += &format!("    fn fragment_{}(&mut self, depth: usize) {{\n", id);

            // Add depth checking to terminate on depth exhaustion
            program += &format!("        if depth >= {} {{ return; }}\n",
                max_depth);

            match fragment {
                Fragment::NonTerminal(options) => {
                    // For non-terminal cases pick a random variant to select
                    // and invoke that fragment's routine
                    program += &format!("        match self.rand() % {} {{\n", options.len());

                    for (option_id, option) in options.iter().enumerate() {
                        program += &format!("            {} => self.fragment_{}(depth + 1),\n", option_id, option.0);
                    }
                    program += &format!("            _ => unreachable!(),\n");

                    program += &format!("        }}\n");
                }
                Fragment::Expression(expr) => {
                    // Invoke all of the expression's routines in order
                    for &exp in expr.iter() {
                        program += &format!("        self.fragment_{}(depth + 1);\n", exp.0);
                    }
                }
                Fragment::Terminal(value) => {
                    // Append the terminal value to the output buffer
                    if SAFE_ONLY {
                        program += &format!("        self.buf.extend_from_slice(&{:?});\n",
                            value);
                    } else {
                        // For some reason this is faster than
                        // `extend_from_slice` even though it does the exact
                        // same thing. This was observed to be over a 4-5x
                        // speedup in some scenarios.
                        program += &format!(r#"
            unsafe {{
                let old_size = self.buf.len();
                let new_size = old_size + {};

                if new_size > self.buf.capacity() {{
                    self.buf.reserve(new_size - old_size);
                }}

                std::ptr::copy_nonoverlapping({:?}.as_ptr(), self.buf.as_mut_ptr().offset(old_size as isize), {});
                self.buf.set_len(new_size);
            }}
    "#, value.len(), value, value.len());
                    }
                }
                Fragment::Nop => {}
            }

            program += "    }\n";
        }
        program += "}\n";

        // Write out the test application
        std::fs::write(path, program)
            .expect("Failed to create output Rust application");
    }
}

fn main() -> std::io::Result<()> {
    // Get access to the command line arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        print!("usage: fzero <grammar json> <output Rust file> <output binary name> <max depth>\n");
        return Ok(());
    }

    // Load up a grammar file
    let grammar: Grammar = serde_json::from_slice(
        &std::fs::read(&args[1])?)?;
    print!("Loaded grammar json\n");

    // Convert the grammar file to the Rust structures
    let mut gram = GrammarRust::new(&grammar);
    print!("Converted grammar to binary format\n");

    // Optimize the grammar
    gram.optimize();
    print!("Optimized grammar\n");

    // Generate a Rust application
    gram.program(&args[2],
        args[4].parse().expect("Invalid digit in max depth"));
    print!("Generated Rust source file\n");

    // Compile the application
    // rustc -O -g test.rs -C target-cpu=native
    let status = Command::new("rustc")
        .arg("-O")                // Optimize the binary
        .arg("-g")                // Generate debug information
        .arg(&args[2])            // Name of the input Rust file
        .arg("-C")                // Optimize for the current microarchitecture
        .arg("target-cpu=native")
        .arg("-o")                // Output filename
        .arg(&args[3]).spawn()?.wait()?;
    assert!(status.success(), "Failed to compile Rust binary");
    print!("Created Rust binary!\n");

    Ok(())
}

