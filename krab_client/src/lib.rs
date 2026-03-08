use wasm_bindgen::prelude::*;
#[cfg(feature = "web")]
use wasm_bindgen::JsCast;
#[cfg(feature = "web")]
use std::panic::{catch_unwind, AssertUnwindSafe};
use web_sys::console;
#[cfg(feature = "web")]
use web_sys::{Element, HtmlElement, Node as WebNode, NodeList};
use krab_core::Node;
#[cfg(feature = "web")]
use std::cell::{Cell, RefCell};
#[cfg(feature = "web")]
use std::rc::Rc;

// Holds event-listener closures for the lifetime of the page so they are not
// dropped (which would invalidate the JS function pointer) but also not
// silently leaked via forget().
#[cfg(feature = "web")]
thread_local! {
    static EVENT_CLOSURES: RefCell<Vec<Closure<dyn FnMut(web_sys::Event)>>> =
        RefCell::new(Vec::new());
}

pub use krab_core::signal::*;

pub mod components;
pub use components::*;

// Function type for creating a component from JSON props
pub type ComponentFactory = fn(props_json: String) -> Node;

pub struct IslandDefinition {
    pub name: &'static str,
    pub factory: ComponentFactory,
}

// Register the inventory
inventory::collect!(IslandDefinition);

#[cfg(feature = "web")]
pub fn log_hydration_diagnostic(scope: &str, name: &str, detail: &str) {
    console::error_1(
        &format!(
            "{{\"scope\":\"{}\",\"island\":\"{}\",\"detail\":\"{}\"}}",
            scope, name, detail
        )
        .into(),
    );
}

#[wasm_bindgen]
pub fn hydrate() {
    console::log_1(&"Hydrating Krab app...".into());
    
    #[cfg(feature = "web")]
    {
        let Some(window) = web_sys::window() else {
            console::error_1(&"{\"scope\":\"hydrate\",\"detail\":\"missing window\"}".into());
            return;
        };
        let Some(document) = window.document() else {
            console::error_1(&"{\"scope\":\"hydrate\",\"detail\":\"missing document\"}".into());
            return;
        };
        
        // Find all elements with data-island attribute
        let islands = match document.query_selector_all("[data-island]") {
            Ok(nodes) => nodes,
            Err(err) => {
                console::error_1(
                    &format!(
                        "{{\"scope\":\"hydrate\",\"detail\":\"query_selector_all failed\",\"error\":\"{:?}\"}}",
                        err
                    )
                    .into(),
                );
                return;
            }
        };
        
        for i in 0..islands.length() {
            let Some(element) = islands.item(i) else {
                console::warn_1(&format!("Missing island element at index {}", i).into());
                continue;
            };
            let Ok(html_element) = element.clone().dyn_into::<HtmlElement>() else {
                console::warn_1(&format!("Island node at index {} is not an HtmlElement", i).into());
                continue;
            };
            
            let Some(name) = html_element.get_attribute("data-island") else {
                console::warn_1(&format!("Island element at index {} missing data-island", i).into());
                continue;
            };
            let props_json = html_element.get_attribute("data-props").unwrap_or_else(|| "{}".to_string());
            
            // Find matching island definition
            let definition = inventory::iter::<IslandDefinition>
                .into_iter()
                .find(|def| def.name == name);
                
            if let Some(def) = definition {
                console::log_1(&format!("Hydrating island: {}", name).into());
                match catch_unwind(AssertUnwindSafe(|| (def.factory)(props_json))) {
                    Ok(node) => {
                        hydrate_node(element.clone().into(), &node);
                        let _ = html_element.set_attribute("data-krab-boundary-state", "ok");
                    }
                    Err(_) => {
                        log_hydration_diagnostic("hydrate", &name, "factory panic captured");
                        let _ = html_element.set_attribute("data-krab-boundary", &name);
                        let _ = html_element.set_attribute("data-krab-boundary-state", "error");
                        html_element.set_inner_html("<div role=\"alert\">Hydration fallback rendered.</div>");
                    }
                }
            } else {
                console::warn_1(&format!("Unknown island: {}", name).into());
            }
        }
    }
}

#[cfg(feature = "web")]
fn hydrate_node(real_node: WebNode, v_node: &Node) {
    // The v_node corresponds to the content of the real_node (the wrapper div)
    hydrate_children(&real_node, &[v_node.clone()]);
}

#[cfg(feature = "web")]
fn hydrate_children(parent: &WebNode, v_nodes: &[Node]) {
    let child_nodes = parent.child_nodes();
    let mut dom_index = 0;
    for v_node in v_nodes {
        dom_index += hydrate_recursive(parent, &child_nodes, dom_index, v_node);
    }
}

#[cfg(feature = "web")]
fn hydrate_recursive(parent: &WebNode, node_list: &NodeList, index: u32, v_node: &Node) -> u32 {
    let real_node_opt = node_list.item(index);

    match v_node {
        Node::Element(v_el) => {
            if let Some(real_node) = real_node_opt {
                let mut match_found = false;
                if let Some(real_el) = real_node.dyn_ref::<Element>() {
                    if real_el.tag_name().to_lowercase() == v_el.tag.to_lowercase() {
                        match_found = true;
                        // Attach events
                        #[cfg(feature = "web")]
                        for event in &v_el.events {
                            let name = event.name.clone();
                            let callback = event.callback.clone();

                            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                                callback(e);
                            }) as Box<dyn FnMut(_)>);

                            if let Err(err) = real_el.add_event_listener_with_callback(&name, closure.as_ref().unchecked_ref()) {
                                console::error_1(
                                    &format!(
                                        "{{\"scope\":\"hydrate_recursive\",\"detail\":\"failed to attach event listener\",\"event\":\"{}\",\"error\":\"{:?}\"}}",
                                        name, err
                                    )
                                    .into(),
                                );
                            }
                            EVENT_CLOSURES.with(|v| v.borrow_mut().push(closure));
                        }
                    }
                }
                
                if match_found {
                    hydrate_children(&real_node, &v_el.children);
                    1 
                } else {
                    console::warn_1(&format!("Hydration mismatch at index {}: expected <{}>, replacing.", index, v_el.tag).into());
                    if let Some(new_node) = create_dom_node(v_node) {
                        if let Err(err) = parent.replace_child(&new_node, &real_node) {
                            console::error_1(
                                &format!(
                                    "{{\"scope\":\"hydrate_recursive\",\"detail\":\"replace_child failed\",\"error\":\"{:?}\"}}",
                                    err
                                )
                                .into(),
                            );
                        }
                    }
                    1
                }
            } else {
                console::warn_1(&format!("Hydration mismatch: expected <{}>, appending.", v_el.tag).into());
                if let Some(new_node) = create_dom_node(v_node) {
                    if let Err(err) = parent.append_child(&new_node) {
                        console::error_1(
                            &format!(
                                "{{\"scope\":\"hydrate_recursive\",\"detail\":\"append_child failed\",\"error\":\"{:?}\"}}",
                                err
                            )
                            .into(),
                        );
                    }
                }
                1
            }
        }
        Node::Text(text) => {
             if let Some(real_node) = real_node_opt {
                 if real_node.node_type() == 3 { // Text node
                     if real_node.text_content().unwrap_or_default() != *text {
                         console::warn_1(&"Text mismatch, fixing...".into());
                         real_node.set_text_content(Some(text));
                     }
                     1
                 } else {
                     console::warn_1(&"Hydration mismatch: expected text, replacing.".into());
                     if let Some(new_node) = create_dom_node(v_node) {
                         if let Err(err) = parent.replace_child(&new_node, &real_node) {
                             console::error_1(
                                 &format!(
                                     "{{\"scope\":\"hydrate_recursive\",\"detail\":\"replace text node failed\",\"error\":\"{:?}\"}}",
                                     err
                                 )
                                 .into(),
                             );
                         }
                     }
                     1
                 }
             } else {
                 console::warn_1(&"Hydration mismatch: expected text, appending.".into());
                 if let Some(new_node) = create_dom_node(v_node) {
                     if let Err(err) = parent.append_child(&new_node) {
                         console::error_1(
                             &format!(
                                 "{{\"scope\":\"hydrate_recursive\",\"detail\":\"append text node failed\",\"error\":\"{:?}\"}}",
                                 err
                             )
                             .into(),
                         );
                     }
                 }
                 1
             }
        }
        Node::Fragment(children) => {
            let mut consumed = 0;
            for child in children {
                consumed += hydrate_recursive(parent, node_list, index + consumed, child);
            }
            consumed
        }
        Node::Dynamic(f) => {
            // Run the function once to get the initial structure (should match SSR)
            // Note: This run is NOT tracked by effect yet.
            let initial_v_node = f();
            
            // Hydrate the initial dynamic output and retain the current root node reference
            // so later reactive updates can replace it.
            
            let consumed = hydrate_recursive(parent, node_list, index, &initial_v_node);

            let current_dom_node = Rc::new(RefCell::new(node_list.item(index)));
            
            // Set up effect for future updates
            let f = f.clone();
            let first_run = Rc::new(Cell::new(true));
            let current_node_ref = current_dom_node.clone();
            
            create_effect(move || {
                let new_v_node = f();
                
                if first_run.get() {
                    first_run.set(false);
                    return;
                }
                
                // Replace the previous DOM node with the newly rendered dynamic output.
                let Some(new_dom_node) = create_dom_node(&new_v_node) else {
                    console::error_1(&"{\"scope\":\"dynamic\",\"detail\":\"failed to create replacement node\"}".into());
                    return;
                };
                
                let old_node = current_node_ref.borrow();
                if let Some(old) = old_node.as_ref() {
                    if let Some(parent) = old.parent_node() {
                        if let Err(err) = parent.replace_child(&new_dom_node, old) {
                            console::error_1(
                                &format!(
                                    "{{\"scope\":\"dynamic\",\"detail\":\"replace_child failed\",\"error\":\"{:?}\"}}",
                                    err
                                )
                                .into(),
                            );
                            return;
                        }
                        *current_node_ref.borrow_mut() = Some(new_dom_node);
                    }
                }
            });
            
            consumed
        }
    }
}

#[cfg(feature = "web")]
fn create_dom_node(v_node: &Node) -> Option<WebNode> {
    let document = web_sys::window().and_then(|w| w.document())?;
    
    match v_node {
        Node::Element(el) => {
             let element = match document.create_element(&el.tag) {
                 Ok(elm) => elm,
                 Err(err) => {
                     console::error_1(
                        &format!(
                            "{{\"scope\":\"create_dom_node\",\"detail\":\"create_element failed\",\"tag\":\"{}\",\"error\":\"{:?}\"}}",
                            el.tag, err
                        )
                        .into(),
                     );
                     return Some(document.create_comment("krab-create-element-error").into());
                 }
             };
             for attr in &el.attributes {
                 if let Err(err) = element.set_attribute(&attr.name, &attr.value) {
                     console::error_1(
                        &format!(
                            "{{\"scope\":\"create_dom_node\",\"detail\":\"set_attribute failed\",\"attribute\":\"{}\",\"error\":\"{:?}\"}}",
                            attr.name, err
                        )
                        .into(),
                     );
                 }
             }
             
             // Attach events for newly created nodes
             #[cfg(feature = "web")]
             for event in &el.events {
                 let name = event.name.clone();
                 let callback = event.callback.clone();
                 let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                     callback(e);
                 }) as Box<dyn FnMut(_)>);
                 if let Err(err) = element.add_event_listener_with_callback(&name, closure.as_ref().unchecked_ref()) {
                     console::error_1(
                        &format!(
                            "{{\"scope\":\"create_dom_node\",\"detail\":\"add_event_listener failed\",\"event\":\"{}\",\"error\":\"{:?}\"}}",
                            name, err
                        )
                        .into(),
                     );
                 }
                 EVENT_CLOSURES.with(|v| v.borrow_mut().push(closure));
             }
             
             for child in &el.children {
                 if let Some(child_node) = create_dom_node(child) {
                     if let Err(err) = element.append_child(&child_node) {
                         console::error_1(
                             &format!(
                                 "{{\"scope\":\"create_dom_node\",\"detail\":\"append child failed\",\"error\":\"{:?}\"}}",
                                 err
                             )
                             .into(),
                         );
                     }
                 }
             }
             Some(element.into())
        }
        Node::Text(text) => {
            Some(document.create_text_node(text).into())
        }
        Node::Fragment(nodes) => {
            let frag = document.create_document_fragment();
            for node in nodes {
                if let Some(child_node) = create_dom_node(node) {
                    if let Err(err) = frag.append_child(&child_node) {
                        console::error_1(
                            &format!(
                                "{{\"scope\":\"create_dom_node\",\"detail\":\"append fragment child failed\",\"error\":\"{:?}\"}}",
                                err
                            )
                            .into(),
                        );
                    }
                }
            }
            Some(frag.into())
        }
        Node::Dynamic(f) => {
            // For nested dynamic nodes in newly created trees
            let anchor = document.create_comment("dynamic-anchor");
            let anchor_node: WebNode = anchor.clone().into();
            
            let current_node = Rc::new(RefCell::new(anchor_node.clone()));
            let f = f.clone();
            
            // Build initial content through an effect so dependency tracking and first render
            // share the same execution path.
            
            let first_run = Rc::new(Cell::new(true));
            
            // `create_effect` runs immediately; capture the first produced node for the
            // synchronous return value while keeping a mutable pointer for later replacements.
            
            let initial_node = Rc::new(RefCell::new(None));
            let initial_node_clone = initial_node.clone();
            
            create_effect(move || {
                let new_v_node = f();
                let Some(new_dom_node) = create_dom_node(&new_v_node) else {
                    console::error_1(&"{\"scope\":\"create_dom_node\",\"detail\":\"dynamic node creation failed\"}".into());
                    return;
                };
                
                if first_run.get() {
                    first_run.set(false);
                    *initial_node_clone.borrow_mut() = Some(new_dom_node.clone());
                    // Store node for subsequent dynamic replacements.
                    *current_node.borrow_mut() = new_dom_node;
                    return;
                }
                
                // Update
                let old = current_node.borrow();
                if let Some(parent) = old.parent_node() {
                    if let Err(err) = parent.replace_child(&new_dom_node, &old) {
                        console::error_1(
                            &format!(
                                "{{\"scope\":\"create_dom_node\",\"detail\":\"dynamic replace_child failed\",\"error\":\"{:?}\"}}",
                                err
                            )
                            .into(),
                        );
                        return;
                    }
                    *current_node.borrow_mut() = new_dom_node;
                }
            });
            
            // Return the initial node produced by the first effect run.
            let result = initial_node.borrow().clone().unwrap_or_else(|| {
                // Defensive fallback for an unexpected empty initial render.
                document.create_comment("empty-dynamic").into()
            });
            
            Some(result)
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console::log_1(&"Krab Client initialized".into());
}
