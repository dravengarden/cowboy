use super::{outline, outline_source, supported, workspace_symbols};

const RUST: &str = "mod outer {\n    pub struct Point { x: i32 }\n    impl Point {\n        fn new() -> Self { todo!() }\n    }\n}\n\nfn 函数() {}\n";

#[test]
fn rust_definitions_nest_by_containment_with_utf16_positions() {
    let symbols = outline_source("src/lib.rs", RUST.as_bytes()).unwrap();
    let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["outer", "函数"]);
    let outer = &symbols[0];
    assert_eq!(outer.kind, 2);
    let inner: Vec<_> = outer.children.iter().map(|s| s.name.as_str()).collect();
    assert!(inner.contains(&"Point"), "{inner:?}");
    let function = &symbols[1];
    assert_eq!(function.selection_start.row, 7);
    assert_eq!(function.selection_start.column, 3);
    assert_eq!(function.selection_end.column, 5, "two UTF-16 units");
}

#[test]
fn every_bundled_grammar_parses_its_own_language() {
    for (path, source, expected) in [
        (
            "a.py",
            "class Greeter:\n    def hello(self):\n        pass\n",
            "Greeter",
        ),
        ("a.go", "package main\nfunc Run() {}\n", "Run"),
        ("a.js", "function render() {}\nclass View {}\n", "render"),
        (
            "a.ts",
            "interface Shape { area(): number }\nfunction draw(s: Shape) {}\n",
            "draw",
        ),
        ("a.tsx", "export function App() { return <div/>; }\n", "App"),
    ] {
        let symbols = outline_source(path, source.as_bytes()).unwrap();
        assert!(
            symbols.iter().any(|symbol| symbol.name == expected),
            "{path}: {symbols:?}"
        );
    }
    assert!(!supported("README.md"));
    assert!(outline_source("README.md", b"# x").is_err());
}

#[test]
fn worktree_reads_stay_inside_the_root_and_symbols_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), RUST).unwrap();
    std::fs::write(root.join("src/other.py"), "def new_point():\n    pass\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.rs"), "fn secret() {}\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("x.rs"), root.join("src/link.rs")).unwrap();
    assert!(outline(root, "src/lib.rs").is_ok());
    for bad in ["../x.rs", "/etc/passwd", "src/link.rs", ""] {
        assert!(outline(root, bad).is_err(), "{bad}");
    }
    let found = workspace_symbols(root, "new", 10).unwrap();
    let mut names: Vec<_> = found
        .symbols
        .iter()
        .map(|s| (s.path.as_str(), s.name.as_str()))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [("src/lib.rs", "new"), ("src/other.py", "new_point")]
    );
    assert!(found.stopped.is_none());
    assert!(
        workspace_symbols(root, "secret", 10)
            .unwrap()
            .symbols
            .is_empty()
    );
    let limited = workspace_symbols(root, "new", 1).unwrap();
    assert_eq!(limited.symbols.len(), 1);
    assert!(limited.stopped.is_some());
}
