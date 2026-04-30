use jupytypst::testkit::{Fixture, KernelCase, KernelInit, Request, run_case};
use typsess::{RenderMode, SourceMode};

#[test]
fn persistent_session_state() {
    insta::assert_snapshot!(
        "persistent_scope",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Code),
            fixtures: vec![],
            requests: vec![
                Request::Execute {
                    input: "let f(a, b) = a + b; let x = 1"
                },
                Request::Execute {
                    input: "f(1, 2) + x"
                },
            ],
        })
    );
    insta::assert_snapshot!(
        "persistent_styles",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Svg, SourceMode::Markup),
            fixtures: vec![],
            requests: vec![
                Request::Execute {
                    input: r#"#set heading(numbering: "I.")
#show: emph
"#
                },
                Request::Execute {
                    input: r#"= test
emphed
            "#
                },
            ],
        })
    );
    insta::assert_snapshot!(
        "persistent_introspector",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Code),
            fixtures: vec![],
            requests: vec![
                Request::Execute {
                    input: r#"let s = state("test", "init")
                s.update("upd")
                context s.get()"#
                },
                Request::Execute {
                    input: "context s.get()"
                },
            ],
        })
    );
}

#[test]
fn page_setup() {
    insta::assert_snapshot!(
        "page_setup_default",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Svg, SourceMode::Markup).with_page_setup(""),
            fixtures: vec![],
            requests: vec![
                Request::Execute {
                    input: r#"#set page(paper: "a4")
                    test in a4"#
                },
                Request::Execute { input: "test" },
            ],
        })
    );
    insta::assert_snapshot!(
        "page_setup_none",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Svg, SourceMode::Markup).with_page_setup(""),
            fixtures: vec![],
            requests: vec![Request::Execute { input: "test" }],
        })
    );
    insta::assert_snapshot!(
        "page_setup_custom",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Svg, SourceMode::Code)
                .with_page_setup("set page(fill: red)"),
            fixtures: vec![],
            requests: vec![Request::Execute { input: "red" }],
        })
    );
}

#[test]
fn multi_page_svg() {
    insta::assert_snapshot!(run_case(KernelCase {
        init: KernelInit::new(RenderMode::Svg, SourceMode::Markup),
        fixtures: vec![],
        requests: vec![Request::Execute {
            input: r#"1
            #pagebreak()
            2"#
        }],
    }));
}

#[test]
fn format_switching_directives() {
    insta::assert_snapshot!(run_case(KernelCase {
        init: KernelInit::new(RenderMode::Svg, SourceMode::Markup),
        fixtures: vec![],
        requests: vec![
            Request::Execute {
                input: "// jupytypst: format=html\ntest"
            },
            Request::Execute {
                input: "// jupytypst: format=svg\ntest"
            },
        ],
    }));
}

#[test]
fn diagnostics() {
    insta::assert_snapshot!(
        "diagnostics_cross_cell",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Markup),
            fixtures: vec![],
            requests: vec![
                Request::Execute {
                    input: "#let f() = panic()"
                },
                Request::Execute {
                    input: "#let g() = f()"
                },
                Request::Execute { input: "#g()" },
            ],
        })
    );
    insta::assert_snapshot!(
        "diagnostics_inside_cell",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Markup),
            fixtures: vec![],
            requests: vec![Request::Execute {
                input: r#"#let f() = panic()
            #let g() = f()
            #g()"#
            }],
        })
    );
    insta::assert_snapshot!(
        "diagnostics_cross_file",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Code),
            fixtures: vec![
                Fixture {
                    path: "defs.typ",
                    contents: "#let f() = panic()",
                },
                Fixture {
                    path: "callbacks.typ",
                    contents: "#let invoke(f) = f()",
                },
            ],
            requests: vec![
                Request::Execute {
                    input: r#"import "defs.typ": f
f()"#
                },
                Request::Execute {
                    input: r#"import "callbacks.typ": invoke
let f = () => panic()
invoke(f)"#
                },
            ],
        })
    );
    insta::assert_snapshot!(
        "diagnostics_error_span",
        run_case(KernelCase {
            init: KernelInit::new(RenderMode::Html, SourceMode::Code),
            fixtures: vec![],
            requests: vec![Request::Execute {
                input: r#"{
    str(1 + 1)
    page.fill
}"#
            }],
        })
    );
}
