use super::colocated_execution_permitted;

#[test]
fn a_declaration_and_a_permission_are_both_required() {
    let permitted: std::collections::BTreeSet<String> = ["hawk".to_owned()].into_iter().collect();
    assert!(colocated_execution_permitted(true, "hawk", &permitted));
    // An enrolled remote Machine cannot read this host by claiming local.
    assert!(!colocated_execution_permitted(true, "falcon", &permitted));
    // A permission is not an assertion: it never converts a remote
    // connection into a local one.
    assert!(!colocated_execution_permitted(false, "hawk", &permitted));
    assert!(!colocated_execution_permitted(false, "falcon", &permitted));
    // With nothing permitted, no Machine may be read on this host.
    let empty = std::collections::BTreeSet::new();
    assert!(!colocated_execution_permitted(true, "hawk", &empty));
}
