use std::cell::RefCell;

use cssparser::{Delimiter, Parser, Token as CssToken};
use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
        states::RawKind,
    },
};

use crate::{
    document::{MAX_DEPTH, MAX_FIELD_BYTES, MAX_FILE_BYTES, MAX_NODES, MAX_TEXT_BYTES},
    outline::{OutlineForest, OutlineNode},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardPayload {
    pub plain: Option<String>,
    pub html: Option<String>,
}

impl ClipboardPayload {
    pub fn validate(&self) -> Result<(), FormatError> {
        let bytes = self
            .plain
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(self.html.as_ref().map_or(0, String::len));
        if bytes > MAX_FILE_BYTES {
            return Err(FormatError(
                "Combined clipboard representations exceed 32 MiB.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineFormat {
    Plain,
    Html,
    OfficeHtml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineWarningKind {
    AmbiguousIndentation,
    UnsupportedOfficeMetadata,
    OutsideListText,
    RepresentationLoss,
    IgnoredContent,
    ConflictingFormats,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineWarning {
    pub category: OutlineWarningKind,
    pub count: usize,
    pub locations: Vec<SourceLocation>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedOutline {
    pub forest: OutlineForest,
    pub source: OutlineFormat,
    pub warnings: Vec<OutlineWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteInterpretation {
    Ready(ParsedOutline),
    Preview(Vec<ParsedOutline>),
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct FormatError(pub String);

pub struct OutlineCopy {
    pub payload: ClipboardPayload,
    pub warnings: Vec<OutlineWarning>,
}

fn warn(
    warnings: &mut Vec<OutlineWarning>,
    category: OutlineWarningKind,
    line: usize,
    column: Option<usize>,
) -> &mut OutlineWarning {
    let index = warnings
        .iter()
        .position(|warning| warning.category == category)
        .unwrap_or_else(|| {
            warnings.push(OutlineWarning {
                category,
                count: 0,
                locations: vec![],
                detail: None,
            });
            warnings.len() - 1
        });
    let warning = &mut warnings[index];
    warning.count += 1;
    if warning.locations.len() < 8 {
        warning.locations.push(SourceLocation { line, column });
    }
    warning
}

struct OpenNode {
    node: OutlineNode,
    literal: bool,
    source_depth: usize,
}

#[derive(Default)]
struct ForestBuilder {
    roots: Vec<OutlineNode>,
    stack: Vec<OpenNode>,
    count: usize,
    text_bytes: usize,
}

impl ForestBuilder {
    fn close_to(&mut self, depth: usize) {
        while self.stack.len() > depth {
            let mut open = self.stack.pop().expect("known open node");
            if !open.literal {
                open.node.label.truncate(
                    open.node
                        .label
                        .trim_end_matches([' ', '\t', '\r', '\n', '\u{c}'])
                        .len(),
                );
            }
            if let Some(parent) = self.stack.last_mut() {
                parent.node.children.push(open.node);
            } else {
                self.roots.push(open.node);
            }
        }
    }

    fn start(
        &mut self,
        depth: usize,
        warnings: &mut Vec<OutlineWarning>,
        line: usize,
    ) -> Result<(), FormatError> {
        if depth == 0 || depth > MAX_DEPTH || self.count >= MAX_NODES {
            return Err(FormatError(
                "Outline exceeds 100,000 nodes or depth 256.".into(),
            ));
        }
        let retained = self
            .stack
            .iter()
            .position(|open| open.source_depth >= depth)
            .unwrap_or(self.stack.len());
        self.close_to(retained);
        if depth
            > self
                .stack
                .last()
                .map_or(1, |parent| parent.source_depth + 1)
        {
            warn(
                warnings,
                OutlineWarningKind::AmbiguousIndentation,
                line,
                None,
            );
        }
        self.count += 1;
        self.stack.push(OpenNode {
            node: OutlineNode {
                label: String::new(),
                children: vec![],
            },
            literal: false,
            source_depth: depth,
        });
        Ok(())
    }

    fn text(&mut self, text: &str, literal: bool) -> Result<(), FormatError> {
        let Some(open) = self.stack.last_mut() else {
            return Ok(());
        };
        if literal {
            if open.node.label.len().saturating_add(text.len()) > MAX_FIELD_BYTES
                || self.text_bytes.saturating_add(text.len()) > MAX_TEXT_BYTES
            {
                return Err(FormatError(
                    "Decoded outline label exceeds 1 MiB or total text exceeds 64 MiB.".into(),
                ));
            }
            open.node.label.push_str(text);
            self.text_bytes += text.len();
            open.literal = true;
        } else {
            for character in text.chars() {
                let character = if matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{c}') {
                    ' '
                } else {
                    character
                };
                if character == ' '
                    && (open.node.label.is_empty() || open.node.label.ends_with(' '))
                {
                    continue;
                }
                let bytes = character.len_utf8();
                if open.node.label.len() + bytes > MAX_FIELD_BYTES
                    || self.text_bytes + bytes > MAX_TEXT_BYTES
                {
                    return Err(FormatError(
                        "Decoded outline label exceeds 1 MiB or total text exceeds 64 MiB.".into(),
                    ));
                }
                open.node.label.push(character);
                self.text_bytes += bytes;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Option<OutlineForest>, FormatError> {
        self.close_to(0);
        if self.roots.is_empty() {
            return Ok(None);
        }
        let forest = OutlineForest { roots: self.roots };
        forest
            .validate()
            .map_err(|error| FormatError(error.to_string()))?;
        Ok(Some(forest))
    }
}

fn strip_marker(text: &str) -> (&str, bool) {
    for marker in ["-", "*", "+", "•", "◦", "▪"] {
        if text == marker {
            return ("", true);
        }
        if let Some(rest) = text
            .strip_prefix(marker)
            .and_then(|rest| rest.strip_prefix([' ', '\t']))
        {
            return (rest, true);
        }
    }
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = text[digits..].strip_prefix(['.', ')'])
    {
        if rest.is_empty() {
            return ("", true);
        }
        if let Some(rest) = rest.strip_prefix([' ', '\t']) {
            return (rest, true);
        }
    }
    (text, false)
}

fn parse_plain(text: &str) -> Result<Option<ParsedOutline>, FormatError> {
    let mut builder = ForestBuilder::default();
    let mut warnings = vec![];
    let mut indentation = Vec::new();
    let mut unit = None;
    let (mut seen_tabs, mut seen_spaces) = (false, false);
    for (index, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
        let prefix = &line[..leading];
        let (label, explicit) = strip_marker(&line[leading..]);
        if label.trim().is_empty() && !explicit {
            continue;
        }
        let tabs = prefix.contains('\t');
        let spaces = prefix.contains(' ');
        if tabs && (spaces || seen_spaces) || spaces && seen_tabs {
            warn(
                &mut warnings,
                OutlineWarningKind::AmbiguousIndentation,
                index + 1,
                Some(1),
            );
        }
        seen_tabs |= tabs;
        seen_spaces |= spaces;
        let column: usize = prefix
            .chars()
            .map(|character| if character == '\t' { 4 } else { 1 })
            .sum();
        if indentation.is_empty() {
            indentation.push(column);
        }
        while indentation.len() > 1 && column < *indentation.last().expect("indentation base") {
            indentation.pop();
        }
        let previous = *indentation.last().expect("indentation base");
        if column > previous {
            let step = column - previous;
            if unit.is_some_and(|unit| unit != step) {
                warn(
                    &mut warnings,
                    OutlineWarningKind::AmbiguousIndentation,
                    index + 1,
                    Some(1),
                );
            }
            unit.get_or_insert(step);
            if indentation.len() >= MAX_DEPTH {
                return Err(FormatError("Plain outline exceeds depth 256.".into()));
            }
            indentation.push(column);
        } else if column != previous {
            warn(
                &mut warnings,
                OutlineWarningKind::AmbiguousIndentation,
                index + 1,
                Some(1),
            );
            indentation[0] = column;
        }
        builder.start(indentation.len(), &mut warnings, index + 1)?;
        builder.text(label, true)?;
    }
    Ok(builder.finish()?.map(|forest| ParsedOutline {
        forest,
        source: OutlineFormat::Plain,
        warnings,
    }))
}

#[derive(Default)]
struct StyleFacts {
    level: Option<usize>,
    office: bool,
    ignore: bool,
    preserve: bool,
}

fn style_facts(style: &str) -> StyleFacts {
    let mut parser = Parser::new(style);
    let mut facts = StyleFacts::default();
    while !parser.is_exhausted() {
        let _: Result<(), cssparser::ParseError<()>> =
            parser.parse_until_before(Delimiter::Semicolon, |value| {
                let property = value.expect_ident_cloned()?;
                value.expect_colon()?;
                let office = property.eq_ignore_ascii_case("mso-list");
                if office {
                    facts.office = true;
                    facts.level = None;
                    facts.ignore = false;
                }
                while !value.is_exhausted() {
                    if let CssToken::Ident(word) = value.next()? {
                        if office {
                            if word.eq_ignore_ascii_case("ignore") {
                                facts.ignore = true;
                            } else if let Some(level) = word.strip_prefix("level") {
                                facts.level = level.parse().ok().filter(|level| *level > 0);
                            }
                        } else if property.eq_ignore_ascii_case("white-space")
                            && matches!(word.as_ref(), "pre" | "pre-wrap" | "break-spaces")
                        {
                            facts.preserve = true;
                        }
                    }
                }
                Ok(())
            });
        let _ = parser.try_parse(Parser::expect_semicolon);
    }
    facts
}

struct Element {
    name: html5ever::LocalName,
    ignore: bool,
    preserve: bool,
    item: bool,
}

struct HtmlBuilder {
    builder: ForestBuilder,
    elements: Vec<Element>,
    lists: Vec<usize>,
    warnings: Vec<OutlineWarning>,
    office: bool,
    list_seen: bool,
    outside: usize,
    outside_pending: bool,
    block_break: bool,
    seeking: bool,
    fragment_seen: bool,
    ended: bool,
    error: Option<FormatError>,
}

impl HtmlBuilder {
    fn new(seeking: bool) -> Self {
        Self {
            builder: ForestBuilder::default(),
            elements: vec![],
            lists: vec![],
            warnings: vec![],
            office: false,
            list_seen: false,
            outside: 0,
            outside_pending: true,
            block_break: false,
            seeking,
            fragment_seen: false,
            ended: false,
            error: None,
        }
    }

    fn token(&mut self, token: Token, line: usize) -> Result<(), FormatError> {
        if let Token::CommentToken(comment) = &token {
            if comment.trim().eq_ignore_ascii_case("StartFragment") && self.seeking {
                self.seeking = false;
                self.fragment_seen = true;
            } else if comment.trim().eq_ignore_ascii_case("EndFragment") && self.fragment_seen {
                self.ended = true;
            }
            return Ok(());
        }
        if self.seeking || self.ended {
            return Ok(());
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind == TagKind::EndTag {
                    let Some(index) = self
                        .elements
                        .iter()
                        .rposition(|element| element.name == tag.name)
                    else {
                        warn(&mut self.warnings, OutlineWarningKind::RepresentationLoss, line, None)
                            .detail = Some("An unmatched HTML closing tag was ignored; review the recovered structure.".into());
                        return Ok(());
                    };
                    let ignored = self.elements.iter().any(|element| element.ignore);
                    if !ignored && matches!(name, "p" | "div") {
                        if self.elements.iter().any(|element| element.item) {
                            self.block_break = true;
                        } else {
                            self.outside_pending = true;
                        }
                    }
                    let removed_lists = self.elements[index..]
                        .iter()
                        .filter(|element| !element.ignore && matches!(&*element.name, "ul" | "ol"))
                        .count();
                    if removed_lists > usize::from(matches!(name, "ul" | "ol")) {
                        warn(&mut self.warnings, OutlineWarningKind::RepresentationLoss, line, None)
                            .detail = Some("Unclosed nested HTML lists were recovered; review the proposed hierarchy.".into());
                    }
                    for _ in 0..removed_lists {
                        if let Some(depth) = self.lists.pop() {
                            self.builder.close_to(depth - 1);
                        }
                    }
                    self.elements.truncate(index);
                    return Ok(());
                }
                let mut style = StyleFacts::default();
                let mut office_class = false;
                for attribute in &tag.attrs {
                    match attribute.name.local.as_ref() {
                        "style" => style = style_facts(&attribute.value),
                        "class" => {
                            office_class = attribute
                                .value
                                .split_ascii_whitespace()
                                .any(|class| class.starts_with("MsoList"))
                        }
                        _ => (),
                    }
                }
                let ignored = self.elements.iter().any(|element| element.ignore);
                let inside_item = self.elements.iter().any(|element| element.item);
                let ignore = ignored
                    || style.ignore
                    || matches!(name, "head" | "script" | "style" | "template");
                if !ignore {
                    match name {
                        "ul" | "ol" => {
                            self.list_seen = true;
                            if self.lists.len() >= MAX_DEPTH {
                                return Err(FormatError("HTML list nesting exceeds 256.".into()));
                            }
                            let depth = if self.elements.iter().any(|element| element.item) {
                                self.builder.stack.len() + 1
                            } else {
                                self.lists.last().copied().unwrap_or(0) + 1
                            };
                            self.lists.push(depth);
                        }
                        "li" => {
                            self.outside_pending = false;
                            self.block_break = false;
                            let boundary = self
                                .elements
                                .iter()
                                .rposition(|element| matches!(element.name.as_ref(), "ul" | "ol"))
                                .map_or(0, |index| index + 1);
                            if let Some(index) =
                                self.elements.iter().enumerate().skip(boundary).find_map(
                                    |(index, element)| (&*element.name == "li").then_some(index),
                                )
                            {
                                self.elements.truncate(index);
                            }
                            let depth = self.lists.last().copied().unwrap_or(1);
                            self.list_seen = true;
                            self.builder.start(depth, &mut self.warnings, line)?;
                        }
                        "p" | "div" if style.level.is_some() || style.office || office_class => {
                            self.outside_pending = false;
                            self.block_break = false;
                            if let Some(level) = style.level {
                                self.office = true;
                                self.list_seen = true;
                                self.builder.start(level, &mut self.warnings, line)?;
                            } else {
                                warn(
                                    &mut self.warnings,
                                    OutlineWarningKind::UnsupportedOfficeMetadata,
                                    line,
                                    None,
                                );
                                self.builder.start(1, &mut self.warnings, line)?;
                            }
                        }
                        "p" | "div" if inside_item => {
                            self.block_break = true;
                        }
                        "p" | "div" if self.lists.is_empty() => {
                            self.outside_pending = true;
                        }
                        "br" => self.builder.text("\n", true)?,
                        "img" | "video" | "audio" | "object" | "iframe" => {
                            warn(
                                &mut self.warnings,
                                OutlineWarningKind::IgnoredContent,
                                line,
                                None,
                            );
                        }
                        _ => (),
                    }
                }
                if !matches!(
                    name,
                    "area"
                        | "base"
                        | "br"
                        | "col"
                        | "embed"
                        | "hr"
                        | "img"
                        | "input"
                        | "link"
                        | "meta"
                        | "param"
                        | "source"
                        | "track"
                        | "wbr"
                ) {
                    if self.elements.len() >= MAX_DEPTH * 4 {
                        return Err(FormatError(
                            "HTML wrapper nesting exceeds the bounded parser stack.".into(),
                        ));
                    }
                    self.elements.push(Element {
                        item: name == "li"
                            || matches!(name, "p" | "div")
                                && (style.level.is_some() || style.office || office_class),
                        name: tag.name,
                        ignore,
                        preserve: style.preserve,
                    });
                }
            }
            Token::CharacterTokens(text) => {
                if !self.elements.iter().any(|element| element.ignore) {
                    if text.trim().is_empty() && (self.outside_pending || self.block_break) {
                        return Ok(());
                    }
                    if (self.builder.stack.is_empty() || self.outside_pending)
                        && !text.trim().is_empty()
                    {
                        self.outside += 1;
                        self.builder.start(1, &mut self.warnings, line)?;
                        self.outside_pending = false;
                        self.block_break = false;
                    }
                    if self.block_break && !text.trim().is_empty() {
                        if self
                            .builder
                            .stack
                            .last()
                            .is_some_and(|open| !open.node.label.is_empty())
                        {
                            self.builder.text("\n", true)?;
                        }
                        self.block_break = false;
                    }
                    let preserve = self.elements.iter().any(|element| element.preserve);
                    self.builder.text(&text, preserve)?;
                }
            }
            Token::NullCharacterToken => {
                return Err(FormatError(
                    "HTML contains an unsupported NUL character.".into(),
                ));
            }
            // HTML5 reports CR references as nonconforming, but preserves the character.
            Token::ParseError(error)
                if error == "Invalid numeric character reference value 0x00000D" => {}
            Token::ParseError(_) => {
                warn(
                    &mut self.warnings,
                    OutlineWarningKind::RepresentationLoss,
                    line,
                    None,
                )
                .detail = Some(
                    "The HTML tokenizer recovered malformed markup; review the proposed outline."
                        .into(),
                );
            }
            _ => (),
        }
        Ok(())
    }
}

struct HtmlSink(RefCell<HtmlBuilder>);

impl TokenSink for HtmlSink {
    type Handle = ();
    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<()> {
        let raw = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => match tag.name.as_ref() {
                "script" => Some(RawKind::ScriptData),
                "style" => Some(RawKind::Rawtext),
                _ => None,
            },
            _ => None,
        };
        let mut builder = self.0.borrow_mut();
        if builder.error.is_none()
            && let Err(error) = builder.token(token, line as usize)
        {
            builder.error = Some(error);
        }
        raw.map_or(TokenSinkResult::Continue, TokenSinkResult::RawData)
    }
}

fn tokenize_html(html: &str, seeking: bool) -> Result<HtmlBuilder, FormatError> {
    let tokenizer = Tokenizer::new(
        HtmlSink(RefCell::new(HtmlBuilder::new(seeking))),
        TokenizerOpts {
            exact_errors: true,
            ..TokenizerOpts::default()
        },
    );
    let queue = BufferQueue::default();
    let mut offset = 0;
    while offset < html.len() {
        let mut end = (offset + 8192).min(html.len());
        while !html.is_char_boundary(end) {
            end -= 1;
        }
        queue.push_back(StrTendril::from_slice(&html[offset..end]));
        let _ = tokenizer.feed(&queue);
        if tokenizer.sink.0.borrow().error.is_some() {
            break;
        }
        offset = end;
    }
    if tokenizer.sink.0.borrow().error.is_none() {
        tokenizer.end();
    }
    let mut builder = tokenizer.sink.0.into_inner();
    if let Some(error) = builder.error.take() {
        return Err(error);
    }
    Ok(builder)
}

fn parse_html(html: &str) -> Result<Option<ParsedOutline>, FormatError> {
    let mut parsed = tokenize_html(html, true)?;
    if !parsed.fragment_seen {
        parsed = tokenize_html(html, false)?;
    }
    if parsed.fragment_seen && !parsed.ended {
        warn(&mut parsed.warnings, OutlineWarningKind::RepresentationLoss, 1, None)
            .detail = Some("StartFragment had no EndFragment; the proposed outline continues to the end of the HTML.".into());
    }
    if parsed.outside > 0 && parsed.list_seen {
        for _ in 0..parsed.outside.min(8) {
            warn(
                &mut parsed.warnings,
                OutlineWarningKind::OutsideListText,
                1,
                None,
            );
        }
        if let Some(warning) = parsed
            .warnings
            .iter_mut()
            .find(|warning| warning.category == OutlineWarningKind::OutsideListText)
        {
            warning.count = parsed.outside;
        }
    }
    Ok(parsed.builder.finish()?.map(|forest| ParsedOutline {
        forest,
        source: if parsed.office {
            OutlineFormat::OfficeHtml
        } else {
            OutlineFormat::Html
        },
        warnings: parsed.warnings,
    }))
}

pub fn parse_clipboard(payload: &ClipboardPayload) -> Result<PasteInterpretation, FormatError> {
    payload.validate()?;
    let rich = payload
        .html
        .as_deref()
        .map(parse_html)
        .transpose()?
        .flatten();
    let plain = payload.plain.as_deref().map(parse_plain).transpose();
    let (mut preferred, alternative) = match (rich, plain) {
        (Some(mut rich), Ok(Some(Some(plain)))) if rich.forest != plain.forest => {
            warn(
                &mut rich.warnings,
                OutlineWarningKind::ConflictingFormats,
                1,
                None,
            );
            (rich, Some(plain))
        }
        (Some(mut rich), Err(error)) => {
            warn(
                &mut rich.warnings,
                OutlineWarningKind::ConflictingFormats,
                1,
                None,
            )
            .detail = Some(error.to_string());
            (rich, None)
        }
        (Some(mut rich), Ok(Some(None))) => {
            warn(
                &mut rich.warnings,
                OutlineWarningKind::ConflictingFormats,
                1,
                None,
            )
            .detail = Some("The plain-text representation contains no outline items.".into());
            (rich, None)
        }
        (Some(rich), _) => (rich, None),
        (None, Ok(Some(Some(mut plain)))) => {
            if payload.html.is_some() {
                warn(
                    &mut plain.warnings,
                    OutlineWarningKind::IgnoredContent,
                    1,
                    None,
                );
            }
            (plain, None)
        }
        (None, Err(error)) => return Err(error),
        _ => {
            return Err(FormatError(
                "The clipboard contains no supported text or list outline.".into(),
            ));
        }
    };
    let preview = alternative.is_some()
        || preferred
            .warnings
            .iter()
            .any(|warning| warning.category != OutlineWarningKind::IgnoredContent)
        || payload.html.is_some() && preferred.source == OutlineFormat::Plain;
    if preview {
        let mut candidates = vec![preferred];
        if let Some(mut alternative) = alternative {
            warn(
                &mut alternative.warnings,
                OutlineWarningKind::ConflictingFormats,
                1,
                None,
            );
            candidates.push(alternative);
        }
        Ok(PasteInterpretation::Preview(candidates))
    } else {
        preferred.warnings.shrink_to_fit();
        Ok(PasteInterpretation::Ready(preferred))
    }
}

pub(crate) struct TextSink {
    pub bytes: usize,
    limit: usize,
    text: Option<String>,
}

impl TextSink {
    pub fn count(limit: usize) -> Self {
        Self {
            bytes: 0,
            limit,
            text: None,
        }
    }
    pub fn output(bytes: usize) -> Self {
        Self {
            bytes: 0,
            limit: bytes,
            text: Some(String::with_capacity(bytes)),
        }
    }
    pub fn push(&mut self, text: &str) -> Result<(), FormatError> {
        self.bytes = self
            .bytes
            .checked_add(text.len())
            .filter(|bytes| *bytes <= self.limit)
            .ok_or_else(|| FormatError("Expanded outline transfer exceeds 32 MiB.".into()))?;
        if let Some(output) = &mut self.text {
            output.push_str(text);
        }
        Ok(())
    }
    pub fn finish(self) -> String {
        self.text.expect("output pass owns allocated text")
    }
}

fn escaped_html(label: &str, output: &mut TextSink) -> Result<(), FormatError> {
    let mut start = 0;
    for (index, character) in label.char_indices() {
        let escape = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\n' => "<br>",
            '\r' => "&#13;",
            '\0' => {
                return Err(FormatError(
                    "HTML cannot preserve a NUL label; the clipboard was not changed.".into(),
                ));
            }
            _ => continue,
        };
        output.push(&label[start..index])?;
        output.push(escape)?;
        start = index + character.len_utf8();
    }
    output.push(&label[start..])
}

fn write_plain(forest: &OutlineForest, output: &mut TextSink) -> Result<(), FormatError> {
    let mut pending: Vec<_> = forest.roots.iter().rev().map(|node| (node, 0)).collect();
    while let Some((node, depth)) = pending.pop() {
        for _ in 0..depth {
            output.push("\t")?;
        }
        output.push("- ")?;
        let mut start = 0;
        for (index, character) in node.label.char_indices() {
            if matches!(character, '\n' | '\r') {
                output.push(&node.label[start..index])?;
                output.push(" ")?;
                start = index + 1;
            }
        }
        output.push(&node.label[start..])?;
        output.push("\n")?;
        pending.extend(node.children.iter().rev().map(|child| (child, depth + 1)));
    }
    Ok(())
}

fn write_html(forest: &OutlineForest, output: &mut TextSink) -> Result<(), FormatError> {
    enum Visit<'a> {
        Node(&'a OutlineNode),
        Close,
    }
    output.push("<ul>")?;
    let mut pending: Vec<_> = forest.roots.iter().rev().map(Visit::Node).collect();
    while let Some(visit) = pending.pop() {
        match visit {
            Visit::Close => output.push("</ul></li>")?,
            Visit::Node(node) => {
                output.push("<li><span style=\"white-space:pre-wrap\">")?;
                escaped_html(&node.label, output)?;
                output.push("</span>")?;
                if node.children.is_empty() {
                    output.push("</li>")?;
                } else {
                    output.push("<ul>")?;
                    pending.push(Visit::Close);
                    pending.extend(node.children.iter().rev().map(Visit::Node));
                }
            }
        }
    }
    output.push("</ul>")
}

pub fn serialize_outline(forest: &OutlineForest) -> Result<OutlineCopy, FormatError> {
    forest
        .validate()
        .map_err(|error| FormatError(error.to_string()))?;
    let mut plain_count = TextSink::count(MAX_FILE_BYTES);
    write_plain(forest, &mut plain_count)?;
    let mut html_count = TextSink::count(MAX_FILE_BYTES - plain_count.bytes);
    write_html(forest, &mut html_count)?;
    let mut plain = TextSink::output(plain_count.bytes);
    let mut html = TextSink::output(html_count.bytes);
    write_plain(forest, &mut plain)?;
    write_html(forest, &mut html)?;
    let mut warnings = vec![];
    let mut pending: Vec<_> = forest.roots.iter().rev().collect();
    let mut index = 0;
    while let Some(node) = pending.pop() {
        index += 1;
        if node.label.contains(['\n', '\r']) || node.label.starts_with(char::is_whitespace) {
            warn(
                &mut warnings,
                OutlineWarningKind::RepresentationLoss,
                index,
                None,
            );
        }
        pending.extend(node.children.iter().rev());
    }
    Ok(OutlineCopy {
        payload: ClipboardPayload {
            plain: Some(plain.finish()),
            html: Some(html.finish()),
        },
        warnings,
    })
}
