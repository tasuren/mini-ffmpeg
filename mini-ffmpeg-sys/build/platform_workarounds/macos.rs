//! macOS SDK half-precision math declarations, excluded because cinrs cannot
//! represent their binary16 ABI. Substitution only permits parsing unused items.

use super::{WorkaroundSet, model::TypeReplacement};

const HALF_TYPE: &str = "_Float16";

pub(super) const SET: WorkaroundSet = WorkaroundSet {
    name: "macOS half-precision math",
    matches_target: |target| target.ends_with("-apple-darwin"),
    replacements: &[TypeReplacement {
        original: HALF_TYPE,
        replacement: "float",
        alias: None,
    }],
    headers: &["math.h"],
    is_sensitive_symbol: excluded_function,
    accepts_declaration: known_prototype,
    excludes_function: excluded_function,
};

/// One unsupported function's original signature and excluded symbol.
struct Declaration {
    name: &'static str,
    arguments: usize,
}

/// macOS SDK math.h declares these functions with binary16 arguments and results.
/// Keep the signature and exclusion together so accepting a declaration cannot
/// accidentally expose its substituted binary32 ABI.
const HALF_FUNCTIONS: &[Declaration] = &[
    Declaration {
        name: "__fabsf16",
        arguments: 1,
    },
    Declaration {
        name: "__hypotf16",
        arguments: 2,
    },
    Declaration {
        name: "__sqrtf16",
        arguments: 1,
    },
    Declaration {
        name: "__ceilf16",
        arguments: 1,
    },
    Declaration {
        name: "__floorf16",
        arguments: 1,
    },
    Declaration {
        name: "__rintf16",
        arguments: 1,
    },
    Declaration {
        name: "__roundf16",
        arguments: 1,
    },
    Declaration {
        name: "__truncf16",
        arguments: 1,
    },
    Declaration {
        name: "__copysignf16",
        arguments: 2,
    },
    Declaration {
        name: "__nextafterf16",
        arguments: 2,
    },
    Declaration {
        name: "__fmaxf16",
        arguments: 2,
    },
    Declaration {
        name: "__fminf16",
        arguments: 2,
    },
    Declaration {
        name: "__fmaf16",
        arguments: 3,
    },
];

fn excluded_function(name: &str) -> bool {
    HALF_FUNCTIONS
        .iter()
        .any(|declaration| declaration.name == name)
}

/// Matches the exact binary16 signature, allowing trailing GNU attributes such
/// as SDK availability metadata. Unsupported types inside attributes still fail.
fn known_prototype(words: &[&str]) -> bool {
    HALF_FUNCTIONS.iter().any(|declaration| {
        let mut expected = vec!["extern", HALF_TYPE, declaration.name, "("];

        for argument in 0..declaration.arguments {
            if argument != 0 {
                expected.push(",");
            }

            expected.push(HALF_TYPE);
        }

        expected.push(")");

        if !words.starts_with(&expected) {
            return false;
        }

        let mut rest = &words[expected.len()..];

        // Skip trailing GNU attributes, such as SDK availability metadata, while
        // checking for unsupported types and references to excluded functions.
        // Attribute contents are otherwise not validated.
        while rest.first() == Some(&"__attribute__") {
            if rest.get(1) != Some(&"(") || rest.get(2) != Some(&"(") {
                return false;
            }

            let mut depth = 0usize;
            let mut end = None;

            for (index, word) in rest.iter().enumerate().skip(1) {
                if *word == HALF_TYPE || HALF_FUNCTIONS.iter().any(|item| item.name == *word) {
                    return false;
                }

                match *word {
                    "(" => depth += 1,
                    ")" => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(index + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }

            let Some(end) = end else {
                return false;
            };
            rest = &rest[end..];
        }

        rest == [";"]
    })
}
