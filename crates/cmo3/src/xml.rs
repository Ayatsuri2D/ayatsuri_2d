//! Minimal XML tree, parser, and a writer that reproduces the editor's JDOM 1.1.3 output
//! (`XMLOutputter` with `Format.getPrettyFormat()`, indent `""`, `TextMode.TRIM`, CRLF).
//!
//! Scope is exactly what main.xml needs: a declaration, processing instructions, elements with
//! attributes, character data, entity references, CDATA (folded into text) and comments
//! (dropped). No namespaces, no DTD processing beyond skipping a DOCTYPE.
//!
//! Whitespace-only text between elements is dropped at parse time; neither writer prints it, so
//! the re-emitted bytes are unchanged and memory stays flat on multi-megabyte files.

use crate::{Result, bail, ensure};

/// A processing instruction `<?target data?>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pi {
    pub target: String,
    pub data: String,
}

impl Pi {
    pub fn new(target: impl Into<String>, data: impl Into<String>) -> Self {
        Pi { target: target.into(), data: data.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub name: String,
    /// Attributes in document order.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Element { name: name.into(), attrs: Vec::new(), children: Vec::new() }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Sets (replacing in place) or appends an attribute, like JDOM's `setAttribute`.
    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((name, value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let i = self.attrs.iter().position(|(k, _)| k == name)?;
        Some(self.attrs.remove(i).1)
    }

    /// Concatenated character data of the direct text children (JDOM `getText`).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            if let Node::Text(t) = c {
                out.push_str(t);
            }
        }
        out
    }

    /// Replaces all content with a single text node (JDOM `setText`). Empty text clears content.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.children.clear();
        let text = text.into();
        if !text.is_empty() {
            self.children.push(Node::Text(text));
        }
    }

    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn child_elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn first_child(&self, name: &str) -> Option<&Element> {
        self.child_elements().find(|e| e.name == name)
    }

    pub fn first_child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.child_elements_mut().find(|e| e.name == name)
    }

    pub fn push(&mut self, child: Element) {
        self.children.push(Node::Element(child));
    }

    pub fn has_element_child(&self) -> bool {
        self.children.iter().any(|c| matches!(c, Node::Element(_)))
    }
}

/// How floats are spelled in element text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatStyle {
    /// `Float.toString`: scientific notation outside 1e-3..1e7 (`2.4414063E-4`).
    Java,
    /// Plain decimal always (`0.00024414063`), as the indented 5.2 writer does.
    Plain,
}

/// How a document is laid out on disk. Two writers are known: the JDOM 1.1.3 pretty format
/// (CRLF, no indentation, `<x />`, trimmed text, a blank line after `</root>`), and the indented
/// form seen in fileFormatVersion 502030002 files (LF, two spaces per level below the
/// `<shared>`/`<main>` sections, `<x/>`, text kept verbatim, plain-decimal floats, no trailing
/// blank line). The layout is detected on parse and replayed on write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub line_separator: String,
    /// One indentation unit (empty for the JDOM shape).
    pub indent: String,
    /// Depths that stay flush: with 1 the root's children (`<shared>`, `<main>`) are not indented
    /// and their children get one unit.
    pub indent_skip: usize,
    /// `true` writes `<x />`, `false` writes `<x/>`.
    pub space_before_empty_close: bool,
    /// Whether one extra line separator follows the document content.
    pub trailing_blank_line: bool,
    /// JDOM TRIM mode: strip leading/trailing whitespace from text. The indented writer keeps it.
    pub trim_text: bool,
    pub float_style: FloatStyle,
}

impl Layout {
    /// The JDOM 1.1.3 `Format.getPrettyFormat()` shape with indent `""` and TRIM text mode.
    pub fn jdom() -> Self {
        Layout {
            line_separator: CRLF.into(),
            indent: String::new(),
            indent_skip: 0,
            space_before_empty_close: true,
            trailing_blank_line: true,
            trim_text: true,
            float_style: FloatStyle::Java,
        }
    }

    /// The indented LF shape observed in Cubism 5.2+ files.
    pub fn indented() -> Self {
        Layout {
            line_separator: LF.into(),
            indent: "  ".into(),
            indent_skip: 1,
            space_before_empty_close: false,
            trailing_blank_line: false,
            trim_text: false,
            float_style: FloatStyle::Plain,
        }
    }

    /// Infers the layout from source bytes. Falls back to [`Layout::jdom`] values per field.
    pub fn detect(bytes: &[u8]) -> Self {
        let mut layout = Layout::jdom();
        layout.line_separator = detect_line_separator(bytes).to_string();

        if let Some(i) = bytes.windows(2).position(|w| w == b"/>") {
            layout.space_before_empty_close = i > 0 && bytes[i - 1] == b' ';
        }

        // Indentation: the `<shared>`/`<main>` line (depth 1) and the line after it (depth 2).
        let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
        fn leading(line: &[u8]) -> &[u8] {
            let n = line.iter().take_while(|b| **b == b' ' || **b == b'\t').count();
            &line[..n]
        }
        let section = lines.iter().position(|l| {
            let t = l.trim_ascii_start();
            t.starts_with(b"<shared>") || t.starts_with(b"<main>")
        });
        if let Some(i) = section {
            let s1 = leading(lines[i]);
            let s2 = lines.get(i + 1).map(|l| leading(l)).unwrap_or(&[]);
            let next_is_close =
                lines.get(i + 1).map(|l| l.trim_ascii_start().starts_with(b"</")).unwrap_or(true);
            if !s1.is_empty() {
                layout.indent = String::from_utf8_lossy(s1).into_owned();
                layout.indent_skip = 0;
            } else if !s2.is_empty() && !next_is_close {
                layout.indent = String::from_utf8_lossy(s2).into_owned();
                layout.indent_skip = 1;
            } else {
                layout.indent.clear();
                layout.indent_skip = 0;
            }
        }

        let sep = layout.line_separator.as_bytes();
        let double: Vec<u8> = [sep, sep].concat();
        layout.trailing_blank_line = bytes.ends_with(&double);

        if !layout.indent.is_empty() {
            layout.trim_text = false;
            layout.float_style = FloatStyle::Plain;
        }
        // Any `dE-d`/`dEd` token means the Java spelling regardless of indentation.
        let has_exponent = bytes.windows(3).any(|w| {
            w[0].is_ascii_digit() && w[1] == b'E' && (w[2].is_ascii_digit() || w[2] == b'-')
        });
        if has_exponent {
            layout.float_style = FloatStyle::Java;
        }
        layout
    }

    fn indent_for(&self, depth: usize) -> String {
        self.indent.repeat(depth.saturating_sub(self.indent_skip))
    }
}

impl Default for Layout {
    fn default() -> Self {
        Layout::jdom()
    }
}

/// A document: processing instructions before the root, the root element, and any PIs after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub prolog: Vec<Pi>,
    pub root: Element,
    pub epilog: Vec<Pi>,
    /// The on-disk layout, detected from the source on parse; fresh documents use [`Layout::jdom`].
    pub layout: Layout,
}

impl Document {
    pub fn new(root: Element) -> Self {
        Document { prolog: Vec::new(), root, epilog: Vec::new(), layout: Layout::jdom() }
    }
}

/// The separator a source file uses: CRLF when its first line break is preceded by CR, else LF.
/// A file with no line break at all gets CRLF.
pub fn detect_line_separator(bytes: &[u8]) -> &'static str {
    match bytes.iter().position(|b| *b == b'\n') {
        Some(i) if i > 0 && bytes[i - 1] == b'\r' => CRLF,
        Some(_) => LF,
        None => CRLF,
    }
}

// ---------------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------------

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

fn is_xml_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Parses UTF-8 XML bytes.
pub fn parse(bytes: &[u8]) -> Result<Document> {
    let mut p = Parser { src: bytes, pos: 0 };
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        p.pos = 3;
    }
    let mut prolog = Vec::new();
    let mut root: Option<Element> = None;
    let mut epilog = Vec::new();

    loop {
        p.skip_space();
        if p.pos >= p.src.len() {
            break;
        }
        ensure!(p.src[p.pos] == b'<', "expected markup at offset {}", p.pos);
        if p.starts_with(b"<?") {
            let pi = p.parse_pi()?;
            // The XML declaration is not a PI for JDOM; it is regenerated on output.
            if pi.target.eq_ignore_ascii_case("xml") {
                continue;
            }
            if root.is_none() { prolog.push(pi) } else { epilog.push(pi) }
        } else if p.starts_with(b"<!--") {
            p.skip_comment()?;
        } else if p.starts_with(b"<!DOCTYPE") {
            p.skip_doctype()?;
        } else {
            ensure!(root.is_none(), "multiple root elements at offset {}", p.pos);
            root = Some(p.parse_element()?);
        }
    }

    match root {
        Some(root) => Ok(Document { prolog, root, epilog, layout: Layout::detect(bytes) }),
        None => bail!("no root element"),
    }
}

impl<'a> Parser<'a> {
    fn starts_with(&self, s: &[u8]) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    fn skip_space(&mut self) {
        while self.pos < self.src.len() && is_xml_space(self.src[self.pos]) {
            self.pos += 1;
        }
    }

    fn find(&self, needle: &[u8]) -> Option<usize> {
        let hay = &self.src[self.pos..];
        hay.windows(needle.len()).position(|w| w == needle).map(|i| self.pos + i)
    }

    fn parse_pi(&mut self) -> Result<Pi> {
        self.pos += 2;
        let end = match self.find(b"?>") {
            Some(e) => e,
            None => bail!("unterminated processing instruction"),
        };
        let body = &self.src[self.pos..end];
        self.pos = end + 2;
        let split = body.iter().position(|b| is_xml_space(*b)).unwrap_or(body.len());
        let target = std::str::from_utf8(&body[..split])
            .map_err(|e| crate::Error::with_source("bad PI target", e))?
            .to_string();
        let mut data_start = split;
        while data_start < body.len() && is_xml_space(body[data_start]) {
            data_start += 1;
        }
        let data = std::str::from_utf8(&body[data_start..])
            .map_err(|e| crate::Error::with_source("bad PI data", e))?
            .to_string();
        Ok(Pi { target, data })
    }

    fn skip_comment(&mut self) -> Result<()> {
        self.pos += 4;
        match self.find(b"-->") {
            Some(e) => {
                self.pos = e + 3;
                Ok(())
            }
            None => bail!("unterminated comment"),
        }
    }

    fn skip_doctype(&mut self) -> Result<()> {
        // Skip to the matching '>' honoring one level of [...] internal subset.
        let mut depth = 0i32;
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => {}
            }
            self.pos += 1;
        }
        bail!("unterminated DOCTYPE")
    }

    fn parse_name(&mut self) -> Result<String> {
        let start = self.pos;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if is_xml_space(b) || matches!(b, b'=' | b'>' | b'/' | b'<' | b'"' | b'\'') {
                break;
            }
            self.pos += 1;
        }
        ensure!(self.pos > start, "expected a name at offset {start}");
        Ok(std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| crate::Error::with_source("bad name", e))?
            .to_string())
    }

    fn parse_element(&mut self) -> Result<Element> {
        debug_assert_eq!(self.src[self.pos], b'<');
        self.pos += 1;
        let name = self.parse_name()?;
        let mut el = Element::new(name);

        // attributes
        loop {
            self.skip_space();
            ensure!(self.pos < self.src.len(), "unterminated start tag <{}>", el.name);
            match self.src[self.pos] {
                b'>' => {
                    self.pos += 1;
                    break;
                }
                b'/' => {
                    ensure!(self.starts_with(b"/>"), "bad empty-element tag <{}>", el.name);
                    self.pos += 2;
                    return Ok(el);
                }
                _ => {
                    let attr_name = self.parse_name()?;
                    self.skip_space();
                    ensure!(
                        self.pos < self.src.len() && self.src[self.pos] == b'=',
                        "expected '=' after attribute {} of <{}>",
                        attr_name,
                        el.name
                    );
                    self.pos += 1;
                    self.skip_space();
                    let quote = self.src.get(self.pos).copied();
                    ensure!(
                        matches!(quote, Some(b'"') | Some(b'\'')),
                        "expected quoted attribute value for {}",
                        attr_name
                    );
                    let quote = quote.unwrap();
                    self.pos += 1;
                    let start = self.pos;
                    while self.pos < self.src.len() && self.src[self.pos] != quote {
                        ensure!(self.src[self.pos] != b'<', "'<' in attribute value {}", attr_name);
                        self.pos += 1;
                    }
                    ensure!(
                        self.pos < self.src.len(),
                        "unterminated attribute value {}",
                        attr_name
                    );
                    let raw = &self.src[start..self.pos];
                    self.pos += 1;
                    el.attrs.push((attr_name, decode_attr_value(raw)?));
                }
            }
        }

        // content
        let mut text_buf: Vec<u8> = Vec::new();
        loop {
            ensure!(self.pos < self.src.len(), "unterminated element <{}>", el.name);
            if self.src[self.pos] == b'<' {
                if self.starts_with(b"</") {
                    let sole = el.children.is_empty();
                    flush_text(&mut text_buf, &mut el, sole)?;
                    self.pos += 2;
                    let close = self.parse_name()?;
                    ensure!(close == el.name, "mismatched </{}> for <{}>", close, el.name);
                    self.skip_space();
                    ensure!(self.src.get(self.pos) == Some(&b'>'), "bad end tag </{}>", close);
                    self.pos += 1;
                    return Ok(el);
                } else if self.starts_with(b"<!--") {
                    self.skip_comment()?;
                } else if self.starts_with(b"<![CDATA[") {
                    self.pos += 9;
                    let end = match self.find(b"]]>") {
                        Some(e) => e,
                        None => bail!("unterminated CDATA"),
                    };
                    // CDATA is folded into the adjacent text, with line ends normalized.
                    push_normalized_lines(&self.src[self.pos..end], &mut text_buf);
                    self.pos = end + 3;
                } else if self.starts_with(b"<?") {
                    // PIs inside elements are not something the editor emits; drop them.
                    self.parse_pi()?;
                } else {
                    flush_text(&mut text_buf, &mut el, false)?;
                    let child = self.parse_element()?;
                    el.children.push(Node::Element(child));
                }
            } else {
                let start = self.pos;
                while self.pos < self.src.len() && self.src[self.pos] != b'<' {
                    self.pos += 1;
                }
                let raw = &self.src[start..self.pos];
                decode_text_into(raw, &mut text_buf)?;
            }
        }
    }
}

/// Appends buffered character data as a text node. Whitespace-only text is dropped unless it is
/// the element's sole content (`keep_blank`), so `<s> </s>` survives for the untrimmed layout
/// while inter-element whitespace never reaches the tree.
fn flush_text(buf: &mut Vec<u8>, el: &mut Element, keep_blank: bool) -> Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let bytes = std::mem::take(buf);
    if !keep_blank && bytes.iter().all(|b| is_xml_space(*b)) {
        return Ok(());
    }
    let s = String::from_utf8(bytes)?;
    match el.children.last_mut() {
        Some(Node::Text(prev)) => prev.push_str(&s),
        _ => el.children.push(Node::Text(s)),
    }
    Ok(())
}

/// Appends `raw` with XML line-end normalization (CRLF and lone CR become LF).
fn push_normalized_lines(raw: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == b'\r' {
            out.push(b'\n');
            if i + 1 < raw.len() && raw[i + 1] == b'\n' {
                i += 1;
            }
        } else {
            out.push(b);
        }
        i += 1;
    }
}

/// Decodes character data: line-end normalization plus entity references.
fn decode_text_into(raw: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        match b {
            b'\r' => {
                out.push(b'\n');
                if i + 1 < raw.len() && raw[i + 1] == b'\n' {
                    i += 1;
                }
            }
            b'&' => {
                let (ch, consumed) = decode_entity(&raw[i..])?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                i += consumed - 1;
            }
            _ => out.push(b),
        }
        i += 1;
    }
    Ok(())
}

/// Attribute-value normalization: CRLF/CR/LF/TAB literals become spaces; entity-derived
/// characters are kept as decoded.
fn decode_attr_value(raw: &[u8]) -> Result<String> {
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        match b {
            b'\r' => {
                out.push(b' ');
                if i + 1 < raw.len() && raw[i + 1] == b'\n' {
                    i += 1;
                }
            }
            b'\n' | b'\t' => out.push(b' '),
            b'&' => {
                let (ch, consumed) = decode_entity(&raw[i..])?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                i += consumed - 1;
            }
            _ => out.push(b),
        }
        i += 1;
    }
    Ok(String::from_utf8(out)?)
}

/// Decodes the entity reference at the start of `s` (which begins with `&`), returning the
/// character and the number of bytes consumed.
fn decode_entity(s: &[u8]) -> Result<(char, usize)> {
    let end = match s.iter().position(|b| *b == b';') {
        Some(e) => e,
        None => bail!("unterminated entity reference"),
    };
    let body = &s[1..end];
    let ch = match body {
        b"lt" => '<',
        b"gt" => '>',
        b"amp" => '&',
        b"quot" => '"',
        b"apos" => '\'',
        _ if body.first() == Some(&b'#') => {
            let (digits, radix) =
                if body.get(1) == Some(&b'x') { (&body[2..], 16) } else { (&body[1..], 10) };
            let text = std::str::from_utf8(digits)
                .map_err(|e| crate::Error::with_source("bad char ref", e))?;
            let code = u32::from_str_radix(text, radix).map_err(|e| {
                crate::Error::with_source(
                    format!("bad character reference &{};", String::from_utf8_lossy(body)),
                    e,
                )
            })?;
            match char::from_u32(code) {
                Some(c) => c,
                None => bail!("character reference out of range: {code}"),
            }
        }
        other => bail!("unknown entity &{};", String::from_utf8_lossy(other)),
    };
    Ok((ch, end + 1))
}

// ---------------------------------------------------------------------------------------------
// Writer (JDOM 1.1.3 pretty format, indent "", TRIM, CRLF)
// ---------------------------------------------------------------------------------------------

pub const CRLF: &str = "\r\n";
pub const LF: &str = "\n";

/// Serializes a document in its [`Layout`].
pub fn write(doc: &Document) -> Vec<u8> {
    let lay = &doc.layout;
    let nl = lay.line_separator.as_str();
    let mut out = String::with_capacity(1 << 16);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    out.push_str(nl);
    for pi in &doc.prolog {
        write_pi(pi, &mut out);
        out.push_str(nl);
    }
    write_element(&doc.root, &mut out, lay, 0);
    out.push_str(nl);
    for pi in &doc.epilog {
        write_pi(pi, &mut out);
        out.push_str(nl);
    }
    if lay.trailing_blank_line {
        out.push_str(nl);
    }
    out.into_bytes()
}

/// Serializes a single element (no declaration, no trailing newline) in the JDOM layout.
pub fn write_element_to_string(el: &Element) -> String {
    let mut out = String::new();
    write_element(el, &mut out, &Layout::jdom(), 0);
    out
}

fn write_pi(pi: &Pi, out: &mut String) {
    out.push_str("<?");
    out.push_str(&pi.target);
    if !pi.data.is_empty() {
        out.push(' ');
        out.push_str(&pi.data);
    }
    out.push_str("?>");
}

fn write_element(el: &Element, out: &mut String, lay: &Layout, depth: usize) {
    let nl = lay.line_separator.as_str();
    out.push('<');
    out.push_str(&el.name);
    for (k, v) in &el.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_attribute(v, out);
        out.push('"');
    }

    // Inter-element whitespace never reaches the tree; a whitespace-only sole text node is
    // JDOM's "all insignificant whitespace" case under TRIM and real content otherwise.
    let significant: Vec<&Node> = el
        .children
        .iter()
        .filter(|c| match c {
            Node::Text(t) => !lay.trim_text || !t.bytes().all(is_xml_space),
            Node::Element(_) => true,
        })
        .collect();
    fn maybe_trim<'t>(lay: &Layout, t: &'t str) -> &'t str {
        if lay.trim_text { trim_ascii_control(t) } else { t }
    }

    if significant.is_empty() {
        out.push_str(if lay.space_before_empty_close { " />" } else { "/>" });
        return;
    }
    out.push('>');

    if el.has_element_child() {
        // Mixed content: every node on its own line at the child indentation.
        let child_indent = lay.indent_for(depth + 1);
        out.push_str(nl);
        let mut first = true;
        for node in &significant {
            if !first {
                out.push_str(nl);
            }
            first = false;
            out.push_str(&child_indent);
            match node {
                Node::Element(child) => write_element(child, out, lay, depth + 1),
                Node::Text(t) => escape_text(maybe_trim(lay, t), out, nl),
            }
        }
        out.push_str(nl);
        out.push_str(&lay.indent_for(depth));
    } else {
        // Text only: no line breaks around it.
        let mut previous: Option<&str> = None;
        for node in &significant {
            if let Node::Text(t) = node {
                if lay.trim_text
                    && let Some(prev) = previous
                    && (ends_with_white(prev) || starts_with_white(t))
                {
                    out.push(' ');
                }
                escape_text(maybe_trim(lay, t), out, nl);
                previous = Some(t);
            }
        }
    }

    out.push_str("</");
    out.push_str(&el.name);
    out.push('>');
}

fn starts_with_white(s: &str) -> bool {
    s.bytes().next().is_some_and(is_xml_space)
}

fn ends_with_white(s: &str) -> bool {
    s.bytes().last().is_some_and(is_xml_space)
}

/// `java.lang.String.trim()`: strips leading and trailing chars with code <= U+0020.
pub fn trim_ascii_control(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

fn escape_text(s: &str, out: &mut String, nl: &str) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\r' => out.push_str("&#xD;"),
            '\n' => out.push_str(nl),
            c if (c as u32) > 0xFFFF => {
                out.push_str("&#x");
                out.push_str(&format!("{:x}", c as u32));
                out.push(';');
            }
            c => out.push(c),
        }
    }
}

fn escape_attribute(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            c if (c as u32) > 0xFFFF => {
                out.push_str("&#x");
                out.push_str(&format!("{:x}", c as u32));
                out.push(';');
            }
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<?version SerializeFormatVersion:2?>\r\n<?import com.live2d.type.CRect?>\r\n<root fileFormatVersion=\"504000000\">\r\n<shared>\r\n<float-array count=\"3\" xs.id=\"#0\" xs.idx=\"0\">1.0 2.5 -3.0E-5</float-array>\r\n</shared>\r\n<main>\r\n<CModelSource isDefaultKeyformLocked=\"false\">\r\n<s xs.n=\"name\">A &amp; B &lt;c&gt; \"q\" &#x1f600;</s>\r\n<s xs.n=\"empty\" />\r\n<null xs.n=\"guid\" />\r\n<float-array xs.n=\"pts\" xs.ref=\"#0\" />\r\n</CModelSource>\r\n</main>\r\n</root>\r\n\r\n";

    #[test]
    fn round_trips_editor_shaped_document() {
        let doc = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(doc.prolog.len(), 2);
        assert_eq!(doc.root.attr("fileFormatVersion"), Some("504000000"));
        let out = write(&doc);
        assert_eq!(String::from_utf8(out).unwrap(), SAMPLE);
    }

    const INDENTED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<?version SerializeFormatVersion:2?>\n<root fileFormatVersion=\"502030002\">\n<shared>\n  <CParameterGuid uuid=\"u\" note=\"n\" xs.id=\"#0\" xs.idx=\"14\"/>\n</shared>\n<main>\n  <CModelSource isDefaultKeyformLocked=\"false\">\n    <s xs.n=\"name\">EXAS</s>\n    <EditorEdition xs.n=\"editorEdition\">\n      <i xs.n=\"edition\">15</i>\n    </EditorEdition>\n    <s xs.n=\"description\"/>\n    <s xs.n=\"localName\"> 18</s>\n    <s xs.n=\"blank\"> </s>\n    <CParameterGuid xs.n=\"g\" xs.ref=\"#0\"/>\n  </CModelSource>\n</main>\n</root>\n";

    #[test]
    fn indented_lf_layout_is_detected_and_replayed() {
        let doc = parse(INDENTED.as_bytes()).unwrap();
        assert_eq!(doc.layout, Layout::indented());
        assert_eq!(String::from_utf8(write(&doc)).unwrap(), INDENTED);
    }

    #[test]
    fn jdom_layout_is_detected() {
        let doc = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(doc.layout, Layout::jdom());
    }

    #[test]
    fn attribute_normalization_and_escaping() {
        let src = b"<a t=\"x\ty\" n=\"l1\nl2\" q=\"it's\" />";
        let doc = parse(src).unwrap();
        assert_eq!(doc.root.attr("t"), Some("x y"));
        assert_eq!(doc.root.attr("n"), Some("l1 l2"));
        let out = String::from_utf8(write(&doc)).unwrap();
        assert!(out.contains("q=\"it&apos;s\""));
    }

    #[test]
    fn text_newlines_become_crlf_and_cr_is_escaped() {
        let src = b"<a>line1\r\nline2&#xD;x</a>";
        let doc = parse(src).unwrap();
        assert_eq!(doc.root.text(), "line1\nline2\rx");
        let out = String::from_utf8(write(&doc)).unwrap();
        assert!(out.contains("<a>line1\r\nline2&#xD;x</a>"));
    }

    #[test]
    fn whitespace_only_text_is_insignificant() {
        let doc = parse(b"<a>\r\n  <b> </b>\r\n</a>").unwrap();
        let out = String::from_utf8(write(&doc)).unwrap();
        assert!(out.contains("<a>\r\n<b />\r\n</a>"));
    }
}
