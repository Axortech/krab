use std::rc::Rc;

pub mod signal;
pub mod resilience;
pub mod config;
pub mod head;
pub mod layout;
pub mod loading;
pub mod render_stream;
pub mod error_boundary;
pub mod style_scope;
pub mod image;
pub mod isr;
pub mod i18n;

#[cfg(not(target_arch = "wasm32"))]
pub mod ws;

#[cfg(feature = "rest")]
pub mod server_fn;

#[cfg(not(target_arch = "wasm32"))]
pub mod telemetry;

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(not(target_arch = "wasm32"))]
pub mod service;

#[cfg(feature = "db")]
pub mod db;

#[cfg(feature = "db")]
pub mod repository;

#[cfg(feature = "rest")]
pub mod http;

#[cfg(feature = "rest")]
pub mod store;

#[cfg(all(feature = "rest", test))]
mod auth_tests;

#[cfg(all(feature = "db", test))]
mod db_tests;

#[cfg(all(feature = "rest", test))]
mod api_tests;

#[cfg(all(feature = "rest", test))]
mod server_fn_tests;

pub trait Render {
    fn render(&self) -> String;
}

impl Render for String {
    fn render(&self) -> String {
        self.clone()
    }
}

impl Render for &str {
    fn render(&self) -> String {
        self.to_string()
    }
}

impl<T: std::fmt::Display> Render for &T {
    fn render(&self) -> String {
        self.to_string()
    }
}

#[derive(Clone)]
pub enum Node {
    Element(Element),
    Text(String),
    Fragment(Vec<Node>),
    Dynamic(Rc<dyn Fn() -> Node>),
}

// Remove Debug derive from Node because Fn is not Debug
impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::Element(e) => f.debug_tuple("Element").field(e).finish(),
            Node::Text(t) => f.debug_tuple("Text").field(t).finish(),
            Node::Fragment(nodes) => f.debug_tuple("Fragment").field(nodes).finish(),
            Node::Dynamic(_) => f.debug_tuple("Dynamic").finish(),
        }
    }
}

impl Render for Node {
    fn render(&self) -> String {
        match self {
            Node::Element(el) => el.render(),
            Node::Text(text) => escape_html_text(text),
            Node::Fragment(nodes) => nodes.iter().map(|n| n.render()).collect(),
            Node::Dynamic(f) => f().render(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Element {
    pub tag: String,
    pub attributes: Vec<Attribute>,
    pub children: Vec<Node>,
    pub events: Vec<EventListener>,
}

#[derive(Clone)]
pub struct EventListener {
    pub name: String,
    #[cfg(feature = "web")]
    pub callback: Rc<dyn Fn(web_sys::Event)>,
    #[cfg(not(feature = "web"))]
    pub callback: (),
}

impl std::fmt::Debug for EventListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventListener")
            .field("name", &self.name)
            .finish()
    }
}

impl Render for Element {
    fn render(&self) -> String {
        let attrs = self.attributes.iter().map(|a| format!(" {}=\"{}\"", a.name, escape_html_attr(&a.value))).collect::<String>();
        let children = self.children.iter().map(|c| c.render()).collect::<String>();
        
        // Note: events are not rendered to HTML string
        
        if self.children.is_empty() {
             match self.tag.as_str() {
                 "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr" => {
                     format!("<{}{}/>", self.tag, attrs)
                 }
                 _ => format!("<{}{}></{}>", self.tag, attrs, self.tag),
             }
        } else {
            format!("<{}{}>{}</{}>", self.tag, attrs, children, self.tag)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

impl Attribute {
    pub fn new(name: String, value: String) -> Self {
        Self { name, value }
    }
}

pub trait IntoNode {
    fn into_node(self) -> Node;
}

impl IntoNode for Node {
    fn into_node(self) -> Node {
        self
    }
}

impl IntoNode for String {
    fn into_node(self) -> Node {
        Node::Text(self)
    }
}

impl IntoNode for &str {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for i32 {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for &i32 {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

// Generic implementation for Closures?
// impl<F> IntoNode for F where F: Fn() -> Node + 'static { ... }
// This might conflict or requires boxing.
// Since we use Rc<dyn Fn() -> Node>, we can impl it.
impl<F> IntoNode for F 
where F: Fn() -> Node + 'static 
{
    fn into_node(self) -> Node {
        Node::Dynamic(Rc::new(self))
    }
}

// Also support closures returning things that can be nodes?
// e.g. Fn() -> String. 
// Rust doesn't support specialization well, so F: Fn() -> Node is safer.
// If the user returns String from closure, they might need to wrap it.
