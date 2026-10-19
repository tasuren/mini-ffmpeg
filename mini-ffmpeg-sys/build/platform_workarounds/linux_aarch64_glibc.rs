//! Reviewed glibc AArch64 vector math declarations (Ubuntu 26.04, glibc 2.43).
//!
//! These scalar replacements only let cinrs parse unused declarations. Neither
//! their aliases nor their functions may survive in the generated public API.
//! Match explicit function families and complete signatures, never all _ZGV names.

use super::{WorkaroundSet, model::TypeReplacement};

pub(super) const SET: WorkaroundSet = WorkaroundSet {
    name: "AArch64 glibc vector math",
    matches_target: |target| target == "aarch64-unknown-linux-gnu",
    replacements: TYPES,
    headers: &["math-vector.h"],
    is_sensitive_symbol: |name| name.starts_with("_ZGV"),
    accepts_declaration: known_declaration,
    excludes_function: |name| signature(name).is_some(),
};

/// Unsupported builtins and the glibc typedefs whose scalar stand-ins are hidden.
const TYPES: &[TypeReplacement] = &[
    TypeReplacement {
        original: "__Float32x4_t",
        replacement: "float",
        alias: Some("__f32x4_t"),
    },
    TypeReplacement {
        original: "__Float64x2_t",
        replacement: "double",
        alias: Some("__f64x2_t"),
    },
    TypeReplacement {
        original: "__SVFloat32_t",
        replacement: "float",
        alias: Some("__sv_f32_t"),
    },
    TypeReplacement {
        original: "__SVFloat64_t",
        replacement: "double",
        alias: Some("__sv_f64_t"),
    },
    TypeReplacement {
        original: "__SVBool_t",
        replacement: "int",
        alias: Some("__sv_bool_t"),
    },
];

// Each listed operation has float/double variants for both NEON and SVE.
// New operations require review; SDKs exposing fewer of these are also accepted.
const UNARY: &[&str] = &[
    "acos", "acosh", "acospi", "asin", "asinh", "asinpi", "atan", "atanh", "atanpi", "cbrt", "cos",
    "cosh", "cospi", "erf", "erfc", "exp", "exp10", "exp2", "expm1", "exp2m1", "exp10m1", "log",
    "log10", "log10p1", "log1p", "log2", "log2p1", "logp1", "rsqrt", "sin", "sinh", "sinpi", "tan",
    "tanh", "tanpi",
];
const BINARY: &[&str] = &["atan2", "atan2pi", "hypot", "pow"];

/// An exact vector ABI family and its original, unmodified C signature.
struct Signature {
    vector: &'static str,
    arguments: usize,
    sve: bool,
}

fn signature(name: &str) -> Option<Signature> {
    let (operation, vector, arguments, sve) = [
        ("_ZGVnN4v_", "__f32x4_t", 1, false),
        ("_ZGVnN4vv_", "__f32x4_t", 2, false),
        ("_ZGVnN2v_", "__f64x2_t", 1, false),
        ("_ZGVnN2vv_", "__f64x2_t", 2, false),
        ("_ZGVsMxv_", "", 1, true),
        ("_ZGVsMxvv_", "", 2, true),
    ]
    .into_iter()
    .find_map(|(prefix, vector, arguments, sve)| {
        name.strip_prefix(prefix)
            .map(|operation| (operation, vector, arguments, sve))
    })?;

    // Double variants use the operation name as-is; float variants append 'f'.
    // Check the operation list because names such as "erf" already end in 'f'.
    let operations = if arguments == 1 { UNARY } else { BINARY };
    let double = operations.contains(&operation);
    let float = operation
        .strip_suffix('f')
        .is_some_and(|base| operations.contains(&base));
    let vector = if sve {
        if double {
            "__sv_f64_t"
        } else if float {
            "__sv_f32_t"
        } else {
            return None;
        }
    } else {
        if !(if vector == "__f32x4_t" { float } else { double }) {
            return None;
        }

        vector
    };

    Some(Signature {
        vector,
        arguments,
        sve,
    })
}

/// Accepts only the five typedefs and 156 reviewed vector prototypes from
/// bits/math-vector.h. NEON requires its vector PCS attribute; SVE requires a
/// final predicate argument. Named parameters and other changes require review.
fn known_declaration(words: &[&str]) -> bool {
    if TYPES
        .iter()
        .any(|rule| words == ["typedef", rule.original, rule.alias.unwrap(), ";"])
    {
        return true;
    }

    let (words, neon_pcs) = if let Some(rest) = words.strip_prefix(&[
        "__attribute__",
        "(",
        "(",
        "__aarch64_vector_pcs__",
        ")",
        ")",
    ]) {
        (rest, true)
    } else {
        (words, false)
    };

    let Some(name) = words.get(1) else {
        return false;
    };
    let Some(signature) = signature(name) else {
        return false;
    };

    if neon_pcs == signature.sve {
        return false;
    }

    let mut expected = vec![signature.vector, name, "("];

    for index in 0..signature.arguments {
        if index != 0 {
            expected.push(",");
        }

        expected.push(signature.vector);
    }

    if signature.sve {
        expected.extend([",", "__sv_bool_t"]);
    }

    expected.extend([")", ";"]);

    words == expected
}
