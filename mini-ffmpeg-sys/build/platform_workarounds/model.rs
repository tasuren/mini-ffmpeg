//! Data shared by platform policies and the target-independent pipeline.

/// A parsing substitution and, optionally, its excluded system typedef.
///
/// The original spelling is also used to recover macro origins and detect C
/// uses. Keeping it here prevents validation from drifting away from replacement.
#[derive(Clone, Copy)]
pub(super) struct TypeReplacement {
    pub original: &'static str,
    pub replacement: &'static str,
    pub alias: Option<&'static str>,
}

/// A coherent policy for one platform header and ABI combination.
///
/// Platform modules own the target predicate, substitutions, reviewed declaration
/// shapes, and excluded functions. The common pipeline owns preprocessing checks
/// and Rust token filtering. Aliases in `replacements` are always excluded.
///
/// Declaration callbacks receive C spellings with substituted types restored
/// through macro origins. A known name alone must never authorize a declaration.
#[derive(Clone, Copy)]
pub struct WorkaroundSet {
    pub(super) name: &'static str,
    pub(super) matches_target: fn(&str) -> bool,
    pub(super) replacements: &'static [TypeReplacement],
    /// Basenames of allowed system headers. An empty list denotes no policy.
    pub(super) headers: &'static [&'static str],
    /// Include unknown members of a reserved symbol family here so they fail
    /// validation instead of silently escaping a reviewed function inventory.
    pub(super) is_sensitive_symbol: fn(&str) -> bool,
    pub(super) accepts_declaration: fn(&[&str]) -> bool,
    pub(super) excludes_function: fn(&str) -> bool,
}
