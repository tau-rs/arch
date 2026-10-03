//! A file's declarations, fingerprinted: what other files can see of it (ADR 0002).
//!
//! A save that changes function bodies only cannot change another file's facts: Rust spells
//! every signature out, so names and types resolve through declarations. Such a save re-analyses
//! the changed file alone. Anything else a file declares may: an item's signature, a `use`, an
//! impl header, a `macro_rules!`, an attribute, an item nested in a body. The fingerprint is a
//! hash of the file's tokens, whitespace and comments (doc comments too) left out, with each
//! function body left out except for the items declared inside it. Two texts with the same
//! fingerprint differ in function bodies, whitespace or comments only. A macro called in a body
//! counts as body: what it expands to is local to the function.
//!
//! Kept on purpose although they hold code: `const fn` bodies (a constant evaluated elsewhere
//! can be a type's length) and the initializers of `const` and `static` items.

use ra_ap_syntax::ast::{self, AstNode};
use ra_ap_syntax::{Edition, NodeOrToken, SyntaxKind, SyntaxNode};
use sha2::{Digest, Sha256};

/// The fingerprint of a Rust file's declarations.
pub fn fingerprint(text: &str) -> String {
    let src = ast::SourceFile::parse(text, Edition::CURRENT).tree();
    let mut hasher = Sha256::new();
    let mut tokens = 0usize;
    for e in src.syntax().descendants_with_tokens() {
        let NodeOrToken::Token(t) = e else { continue };
        let doc = matches!(
            t.kind(),
            SyntaxKind::OUTER_DOC_COMMENT | SyntaxKind::INNER_DOC_COMMENT
        );
        if t.kind().is_trivia() || doc || in_body(t.parent()) {
            continue;
        }
        hasher.update(t.text().as_bytes());
        hasher.update([0]);
        tokens += 1;
    }
    hasher.update(tokens.to_le_bytes());
    hex::encode(hasher.finalize())
}

/// Whether a token under `node` sits in a function body and outside every item declared there.
fn in_body(node: Option<SyntaxNode>) -> bool {
    let mut at = node;
    while let Some(n) = at {
        if n.kind() == SyntaxKind::BLOCK_EXPR
            && let Some(f) = n.parent().and_then(ast::Fn::cast)
            && f.body().is_some_and(|b| b.syntax() == &n)
        {
            return f.const_token().is_none();
        }
        // A macro call parses as an item anywhere; in a body, what it expands to is local.
        if ast::Item::can_cast(n.kind()) && n.kind() != SyntaxKind::MACRO_CALL {
            return false;
        }
        at = n.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::fingerprint;

    fn same(a: &str, b: &str) -> bool {
        fingerprint(a) == fingerprint(b)
    }

    const BASE: &str = "use crate::x::Y;\n\npub fn f(a: u32) -> u32 {\n    a + 1\n}\n";

    #[test]
    fn bodies_whitespace_and_comments_are_not_declarations() {
        assert!(same(
            BASE,
            "use crate::x::Y;\n\npub fn f(a: u32) -> u32 {\n    // twice\n    let b = a * 2;\n    b + other::call()\n}\n"
        ));
        assert!(same(
            BASE,
            "use crate::x::Y;   // y\n/// docs\npub fn f(a: u32)\n    -> u32 { a + 1 }\n"
        ));
        // A method body, a closure in a body, a macro call in a body.
        assert!(same(
            "impl S { fn m(&self) { self.a() } }",
            "impl S { fn m(&self) { let c = || println!(\"x\"); c() } }"
        ));
    }

    #[test]
    fn what_other_files_can_see_is() {
        for changed in [
            // A signature.
            "use crate::x::Y;\n\npub fn f(a: u64) -> u32 {\n    a + 1\n}\n",
            // A use.
            "use crate::x::Z;\n\npub fn f(a: u32) -> u32 {\n    a + 1\n}\n",
            // Visibility.
            "use crate::x::Y;\n\nfn f(a: u32) -> u32 {\n    a + 1\n}\n",
            // An attribute.
            "use crate::x::Y;\n\n#[get(\"/x\")]\npub fn f(a: u32) -> u32 {\n    a + 1\n}\n",
            // An item nested in a body.
            "use crate::x::Y;\n\npub fn f(a: u32) -> u32 {\n    fn g() {}\n    a + 1\n}\n",
            // A use in a body.
            "use crate::x::Y;\n\npub fn f(a: u32) -> u32 {\n    use std::fmt;\n    a + 1\n}\n",
            // A macro_rules! in a body.
            "use crate::x::Y;\n\npub fn f(a: u32) -> u32 {\n    macro_rules! m { () => {} }\n    a + 1\n}\n",
        ] {
            assert!(!same(BASE, changed), "{changed}");
        }
        assert!(!same("impl A for S {}", "impl B for S {}"));
        assert!(!same(
            "macro_rules! m { () => { 1 } }",
            "macro_rules! m { () => { 2 } }"
        ));
        assert!(!same("const N: usize = 1;", "const N: usize = 2;"));
        assert!(!same(
            "const fn n() -> usize { 1 }",
            "const fn n() -> usize { 2 }"
        ));
        // A nested item's own body is a body.
        assert!(same(
            "fn f() { fn g() -> u8 { 1 } }",
            "fn f() { fn g() -> u8 { 2 } }"
        ));
    }
}
