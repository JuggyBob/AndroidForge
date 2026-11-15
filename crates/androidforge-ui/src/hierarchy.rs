use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if !value.starts_with('[') {
            return None;
        }
        let mut numbers = Vec::new();
        let mut current = String::new();
        for ch in value.chars() {
            if ch.is_ascii_digit() || ch == '-' {
                current.push(ch);
            } else if !current.is_empty() {
                numbers.push(current.parse::<i32>().ok()?);
                current.clear();
            }
        }
        if !current.is_empty() {
            numbers.push(current.parse::<i32>().ok()?);
        }
        if numbers.len() != 4 {
            return None;
        }
        Some(Self {
            left: numbers[0],
            top: numbers[1],
            right: numbers[2],
            bottom: numbers[3],
        })
    }

    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn area(&self) -> i64 {
        i64::from(self.width()) * i64::from(self.height())
    }

    pub fn center(&self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    pub fn intersects(&self, width: u32, height: u32) -> bool {
        self.right > 0
            && self.bottom > 0
            && self.left < width as i32
            && self.top < height as i32
            && self.area() > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiNode {
    pub id: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<usize>,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    /// Some UIAutomator builds expose an EditText hint/placeholder. When they
    /// do, it is separate from real user text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    pub bounds: Rect,
    pub bounds_raw: String,
    pub clickable: bool,
    pub long_clickable: bool,
    pub enabled: bool,
    pub focused: bool,
    pub selected: bool,
    pub scrollable: bool,
    pub checked: bool,
    pub password: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_attr: Option<String>,
    pub path: String,
}

impl UiNode {
    pub fn visible(&self) -> bool {
        self.displayed != Some(false) && self.bounds.area() > 0
    }

    pub fn editable_like(&self) -> bool {
        let class = self.class.as_deref().unwrap_or("").to_ascii_lowercase();
        class.contains("edittext")
            || class.contains("textfield")
            || class.contains("autocompletetextview")
            || class.contains("searchview")
    }

    /// Text is empty, or exactly matches an explicit hint/placeholder.
    /// Safe to use before typing: a real value that happens to equal a
    /// content description is not treated as empty.
    pub fn text_is_hint(&self) -> bool {
        let Some(text) = self.text.as_deref() else {
            return true;
        };
        text.is_empty() || self.hint.as_deref().is_some_and(|hint| hint == text)
    }

    /// Text should be treated as empty after a clear attempt. Android Settings
    /// and some OEM fields expose their hint as `text` after the real contents
    /// are gone; a few expose the field label as text instead. Only editable
    /// fields can use the content-description fallback.
    pub fn text_is_placeholder_after_clear(&self) -> bool {
        if self.text_is_hint() {
            return true;
        }
        let Some(text) = self.text.as_deref() else {
            return true;
        };
        self.editable_like()
            && self
                .content_desc
                .as_deref()
                .is_some_and(|description| description == text)
    }

    pub fn short_label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(text) = &self.text {
            parts.push(format!("text={text:?}"));
        }
        if let Some(desc) = &self.content_desc {
            parts.push(format!("desc={desc:?}"));
        }
        if let Some(id) = &self.resource_id {
            parts.push(format!("id={id:?}"));
        }
        if let Some(class) = &self.class {
            parts.push(format!("class={class:?}"));
        }
        parts.push(format!(
            "bounds=[{},{}][{},{}]",
            self.bounds.left, self.bounds.top, self.bounds.right, self.bounds.bottom
        ));
        if self.clickable {
            parts.push("clickable=true".to_string());
        }
        if self.focused {
            parts.push("focused=true".to_string());
        }
        if self.password {
            parts.push("password=true".to_string());
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiHierarchy {
    pub xml: String,
    pub rotation: Option<i32>,
    pub nodes: Vec<UiNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<usize>,
    pub package_name: Option<String>,
}

impl UiHierarchy {
    pub fn parse(xml: &str) -> Result<Self> {
        let document = roxmltree::Document::parse(xml).map_err(|error| {
            ForgeError::new(
                ErrorCode::UiHierarchyInvalid,
                format!("UI hierarchy XML is invalid: {error}"),
            )
            .with_suggestion(
                "Retry the UI dump. UIAutomator occasionally writes a partial hierarchy while the screen is changing.",
            )
        })?;
        let root_element = document.root_element();
        let rotation = root_element
            .attribute("rotation")
            .and_then(|value| value.parse::<i32>().ok());
        let mut nodes = Vec::new();
        let mut synthetic_root = None;
        let mut package_name = None;
        for child in root_element.children().filter(|child| child.is_element()) {
            if child.tag_name().name() == "node" {
                let id = add_node(child, None, 0, &mut nodes);
                if synthetic_root.is_none() {
                    synthetic_root = Some(id);
                }
            }
        }
        if let Some(id) = synthetic_root {
            package_name = nodes[id].package.clone();
        }
        // If the document root itself is a node (unusual but valid), include it.
        if nodes.is_empty() && root_element.tag_name().name() == "node" {
            let id = add_node(root_element, None, 0, &mut nodes);
            synthetic_root = Some(id);
            package_name = nodes[id].package.clone();
        }
        if nodes.is_empty() {
            return Err(ForgeError::new(
                ErrorCode::UiHierarchyInvalid,
                "UI hierarchy XML contained no <node> elements",
            )
            .with_suggestion("Retry the dump or inspect the current screen."));
        }
        Ok(Self {
            xml: xml.to_string(),
            rotation,
            nodes,
            root: synthetic_root,
            package_name,
        })
    }

    pub fn focused(&self) -> Vec<&UiNode> {
        self.nodes.iter().filter(|node| node.focused).collect()
    }

    pub fn node(&self, id: usize) -> Option<&UiNode> {
        self.nodes.get(id)
    }

    pub fn clickable_ancestor(&self, node: &UiNode) -> Option<&UiNode> {
        let mut current = node.parent;
        while let Some(id) = current {
            let candidate = self.nodes.get(id)?;
            if candidate.clickable && candidate.bounds.area() > 0 {
                return Some(candidate);
            }
            current = candidate.parent;
        }
        None
    }
}

fn add_node(
    xml_node: roxmltree::Node<'_, '_>,
    parent: Option<usize>,
    depth: usize,
    nodes: &mut Vec<UiNode>,
) -> usize {
    let id = nodes.len();
    let path = match parent {
        Some(parent) => format!("{}.{id}", nodes[parent].path),
        None => id.to_string(),
    };
    let bounds_raw = xml_node.attribute("bounds").unwrap_or("").to_string();
    let node = UiNode {
        id,
        parent,
        children: Vec::new(),
        depth,
        text: non_empty(xml_node.attribute("text")),
        content_desc: non_empty(xml_node.attribute("content-desc")),
        hint: non_empty(
            xml_node
                .attribute("hint")
                .or_else(|| xml_node.attribute("hint-text")),
        ),
        resource_id: non_empty(xml_node.attribute("resource-id")),
        class: non_empty(xml_node.attribute("class")),
        package: non_empty(xml_node.attribute("package")),
        bounds: Rect::parse(&bounds_raw).unwrap_or_default(),
        bounds_raw,
        clickable: bool_attr(xml_node.attribute("clickable")),
        long_clickable: bool_attr(xml_node.attribute("long-clickable")),
        enabled: bool_attr_default(xml_node.attribute("enabled"), true),
        focused: bool_attr(xml_node.attribute("focused")),
        selected: bool_attr(xml_node.attribute("selected")),
        scrollable: bool_attr(xml_node.attribute("scrollable")),
        checked: bool_attr(xml_node.attribute("checked")),
        password: bool_attr(xml_node.attribute("password")),
        displayed: xml_node
            .attribute("displayed")
            .and_then(|value| value.parse::<bool>().ok()),
        index_attr: xml_node.attribute("index").map(str::to_string),
        path,
    };
    nodes.push(node);
    for child in xml_node.children().filter(|child| child.is_element()) {
        if child.tag_name().name() == "node" {
            let child_id = add_node(child, Some(id), depth + 1, nodes);
            nodes[id].children.push(child_id);
        }
    }
    id
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bool_attr(value: Option<&str>) -> bool {
    value
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn bool_attr_default(value: Option<&str>, default: bool) -> bool {
    value
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.example" content-desc="" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[0,0][1080,2400]">
    <node index="0" text="Save" resource-id="com.example:id/save" class="android.widget.Button" package="com.example" content-desc="Save changes" clickable="true" enabled="true" focused="false" scrollable="false" bounds="[100,200][300,300]" />
    <node index="1" text="" resource-id="com.example:id/name" class="android.widget.EditText" package="com.example" content-desc="Name" clickable="true" enabled="true" focused="true" scrollable="false" password="false" bounds="[100,400][900,500]" />
  </node>
</hierarchy>"#;

    #[test]
    fn parses_hierarchy_and_ancestors() {
        let hierarchy = UiHierarchy::parse(XML).unwrap();
        assert_eq!(hierarchy.nodes.len(), 3);
        assert_eq!(hierarchy.package_name.as_deref(), Some("com.example"));
        let save = &hierarchy.nodes[1];
        assert_eq!(save.text.as_deref(), Some("Save"));
        assert_eq!(save.bounds.center(), (200, 250));
        assert_eq!(save.bounds.area(), 20000);
        assert!(save.visible());
        let editable = hierarchy.nodes.iter().find(|node| node.focused).unwrap();
        assert!(editable.editable_like());
        assert_eq!(
            hierarchy
                .clickable_ancestor(&hierarchy.nodes[0])
                .map(|node| node.id),
            None
        );
    }

    #[test]
    fn hint_and_content_desc_mark_placeholder_text() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="Search settings" resource-id="com.example:id/search" class="android.widget.EditText" package="com.example" content-desc="Search settings" hint="Search settings" clickable="true" enabled="true" focused="true" scrollable="false" bounds="[0,0][900,100]" />
  <node index="1" text="typed query" resource-id="com.example:id/search" class="android.widget.EditText" package="com.example" content-desc="Search settings" hint="Search settings" clickable="true" enabled="true" focused="true" scrollable="false" bounds="[0,100][900,200]" />
</hierarchy>"#;
        let hierarchy = UiHierarchy::parse(xml).unwrap();
        assert!(hierarchy.nodes[0].text_is_hint());
        assert!(hierarchy.nodes[0].text_is_placeholder_after_clear());
        assert!(!hierarchy.nodes[1].text_is_hint());
        assert!(!hierarchy.nodes[1].text_is_placeholder_after_clear());

        let mut content_desc_only = hierarchy.nodes[1].clone();
        content_desc_only.hint = None;
        content_desc_only.text = Some("Search settings".to_string());
        content_desc_only.content_desc = Some("Search settings".to_string());
        assert!(!content_desc_only.text_is_hint());
        assert!(content_desc_only.text_is_placeholder_after_clear());
    }

    #[test]
    fn parses_bounds_robustly() {
        assert_eq!(
            Rect::parse("[10,20][30,40]").unwrap(),
            Rect {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40
            }
        );
        assert!(Rect::parse("garbage").is_none());
    }

    #[test]
    fn rejects_invalid_xml() {
        let error = UiHierarchy::parse("<hierarchy><node></hierarchy>").unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiHierarchyInvalid);
    }
}
