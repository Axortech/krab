/// Integration tests for the `view!` macro.
///
/// These tests verify the *runtime behaviour* of the expansion (the rendered
/// HTML output), which implicitly validates that the macro emitted correct code.
/// Compile-error cases are covered by the `trybuild` suite in `tests/compile_fail/`.
use krab_core::Render;
use krab_macros::view;

#[test]
fn self_closing_element_renders_correctly() {
    let html = view! { <br/> }.render();
    assert_eq!(html, "<br/>");
}

#[test]
fn element_with_no_children_renders_open_close_tags() {
    let html = view! { <div></div> }.render();
    assert_eq!(html, "<div></div>");
}

#[test]
fn element_with_text_child() {
    let html = view! { <p>"Hello"</p> }.render();
    assert_eq!(html, "<p>Hello</p>");
}

#[test]
fn element_with_string_attribute() {
    let html = view! { <div id="main"></div> }.render();
    assert_eq!(html, "<div id=\"main\"></div>");
}

#[test]
fn element_with_expression_attribute() {
    let cls = "active";
    let html = view! { <span class={cls}></span> }.render();
    assert_eq!(html, "<span class=\"active\"></span>");
}

#[test]
fn nested_elements_render_correctly() {
    let html = view! {
        <div>
            <h1>"Title"</h1>
            <p>"Body"</p>
        </div>
    }
    .render();
    assert_eq!(html, "<div><h1>Title</h1><p>Body</p></div>");
}

#[test]
fn expression_child_with_integer() {
    let count = 42i32;
    let html = view! { <span>{count}</span> }.render();
    assert_eq!(html, "<span>42</span>");
}

#[test]
fn expression_child_with_string_variable() {
    let name = "Krab";
    let html = view! { <b>{name}</b> }.render();
    assert_eq!(html, "<b>Krab</b>");
}

#[test]
fn fragment_renders_children_without_wrapper() {
    let html = view! {
        <>
            <span>"A"</span>
            <span>"B"</span>
        </>
    }
    .render();
    assert_eq!(html, "<span>A</span><span>B</span>");
}

#[test]
fn html_text_content_is_escaped() {
    let user_input = "<script>alert('xss')</script>";
    let html = view! { <p>{user_input}</p> }.render();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn attribute_value_is_escaped() {
    let val = r#""><img src=x onerror=alert(1)>"#;
    let html = view! { <div title={val}></div> }.render();
    assert!(!html.contains("<img"));
    assert!(html.contains("&quot;"));
}

#[test]
fn void_elements_do_not_double_close() {
    for tag in &["area", "base", "br", "col", "embed", "hr", "img", "input",
                  "link", "meta", "param", "source", "track", "wbr"] {
        // We can't call view! with a variable tag name, so test via Node directly
        let el = krab_core::Node::Element(krab_core::Element {
            tag: tag.to_string(),
            attributes: vec![],
            children: vec![],
            events: vec![],
        });
        let rendered = el.render();
        assert!(
            rendered.ends_with("/>"),
            "void element <{tag}> should self-close, got: {rendered}"
        );
    }
}
