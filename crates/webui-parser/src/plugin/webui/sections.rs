// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Borrowed source-range compilation with suspended section frames.

use super::*;
use crate::named_fragments::FragmentDeclarations;

struct SectionCursor {
    position: usize,
    end: usize,
    destination: Option<usize>,
    key_skip: Option<usize>,
    raw_end: usize,
    table_closes: Vec<(usize, &'static str)>,
}

struct SectionFrame {
    cursor: SectionCursor,
    meta: TemplateSectionMeta,
}

impl SectionFrame {
    fn new(range: Range<usize>, destination: Option<usize>, key_skip: Option<usize>) -> Self {
        Self {
            cursor: SectionCursor {
                position: range.start,
                end: range.end,
                destination,
                key_skip,
                raw_end: 0,
                table_closes: Vec::new(),
            },
            meta: TemplateSectionMeta {
                html: String::with_capacity(range.len().min(4096)),
                ..TemplateSectionMeta::default()
            },
        }
    }
}

fn push_suspended_cursor(stack: &mut Vec<SectionCursor>, cursor: SectionCursor) {
    if stack.capacity() == 0 {
        stack.reserve_exact(1);
    }
    stack.push(cursor);
}

#[cold]
#[inline(never)]
fn invalid_section_continuation() -> crate::ParserError {
    crate::ParserError::Parse(
        "nested template section lost its destination or parent; report this as a WebUI compiler bug"
            .to_string(),
    )
}

struct SectionCompiler<'a> {
    component: &'a str,
    source: &'a str,
    declarations: Option<&'a FragmentDeclarations>,
    blocks: Vec<TemplateSectionMeta>,
}

pub(super) fn compile_to_metadata(
    component: &str,
    input: &str,
    mut root_events: Vec<EventBinding>,
) -> Result<TemplateMeta> {
    // Discovery must see boundary tags and the owning wrapper before either
    // disappears: otherwise an illegal nested declaration could become a root.
    let declarations = FragmentDeclarations::collect(component, input, true, false)?
        .filter(|graph| !graph.declarations.is_empty());
    let stripped;
    let (source, root_body) = if let Some(declarations) = &declarations {
        if declarations.root_body.start != 0 && root_events.is_empty() {
            root_events = extract_root_events(component, input)?;
        }
        (input, declarations.root_body.clone())
    } else {
        stripped = WebUIParserPlugin::strip_boundary_directive_tags(input);
        let trimmed = stripped.trim();
        let body = shadow_template_body(trimmed).unwrap_or(trimmed);
        (body, 0..body.len())
    };
    let declaration_count = declarations
        .as_ref()
        .map_or(0, |graph| graph.declarations.len());
    let mut compiler = SectionCompiler {
        component,
        source,
        declarations: declarations.as_ref(),
        blocks: Vec::with_capacity(declaration_count),
    };
    compiler
        .blocks
        .resize_with(declaration_count, TemplateSectionMeta::default);
    let root = compiler.compile(root_body)?;
    if let Some(declarations) = &declarations {
        // Validate every body, including unreachable declarations, exactly once.
        // A render records an edge; it never schedules or expands its target.
        for (index, declaration) in declarations.declarations.iter().enumerate() {
            let block = compiler.compile(declaration.body.clone())?;
            compiler.blocks[index] = block;
        }
    }
    Ok(TemplateMeta {
        root,
        blocks: compiler.blocks,
        root_events,
        declaration_count,
    })
}

impl SectionCompiler<'_> {
    fn compile(&mut self, range: Range<usize>) -> Result<TemplateSectionMeta> {
        let mut frame = SectionFrame::new(range, None, None);
        let mut suspended = Vec::new();
        loop {
            while let Some(&(end, tag)) = frame.cursor.table_closes.last() {
                if end > frame.cursor.position {
                    break;
                }
                let _ = write!(frame.meta.html, "</{tag}>");
                frame.cursor.table_closes.pop();
            }
            if frame.cursor.position < frame.cursor.end {
                if let Some(child) = self.advance(&mut frame)? {
                    let destination = child
                        .cursor
                        .destination
                        .ok_or_else(invalid_section_continuation)?;
                    // A new child slot is unread until completion. Park the parent's
                    // metadata there so suspended cursors do not duplicate its headers.
                    self.blocks[destination] = frame.meta;
                    push_suspended_cursor(&mut suspended, frame.cursor);
                    frame = child;
                }
                continue;
            }
            if frame.cursor.destination.is_none() {
                debug_assert!(suspended.is_empty());
                drop(std::mem::take(&mut suspended));
            }
            finalize_template_section(&mut frame.meta);
            if let Some(index) = frame.cursor.destination {
                let meta = std::mem::replace(&mut self.blocks[index], frame.meta);
                frame = SectionFrame {
                    cursor: suspended.pop().ok_or_else(invalid_section_continuation)?,
                    meta,
                };
            } else {
                return Ok(frame.meta);
            }
        }
    }

    fn advance(&mut self, frame: &mut SectionFrame) -> Result<Option<SectionFrame>> {
        let remaining = &self.source[frame.cursor.position..frame.cursor.end];
        if remaining.starts_with('<') {
            return self.advance_markup(frame);
        }
        if let Some(next) = compile_text_binding_at(
            &self.source[..frame.cursor.end],
            frame.cursor.position,
            &mut frame.meta,
        ) {
            frame.cursor.position = next;
            return Ok(None);
        }
        self.compile_literal(frame)?;
        Ok(None)
    }

    fn advance_markup(&mut self, frame: &mut SectionFrame) -> Result<Option<SectionFrame>> {
        if let Some(declarations) = self.declarations {
            if let Ok(index) = declarations
                .table_wrappers
                .binary_search_by_key(&frame.cursor.position, |wrapper| wrapper.range.start)
            {
                let wrapper = &declarations.table_wrappers[index];
                let _ = write!(frame.meta.html, "<{}>", wrapper.tag);
                frame
                    .cursor
                    .table_closes
                    .push((wrapper.range.end, wrapper.tag));
            }
        }
        let remaining = &self.source[frame.cursor.position..frame.cursor.end];
        if remaining.starts_with("<!--") {
            if let Some(close) = find_comment_close(remaining) {
                frame.cursor.position += close;
                return Ok(None);
            }
        }
        if self.compile_named_directive(frame) {
            return Ok(None);
        }
        let opening_name = parse_tag(remaining)
            .filter(|tag| !tag.closing)
            .map(|tag| tag.name);
        if opening_name == Some("if") {
            if let Some((condition, body, consumed)) = parse_if_block(
                self.component,
                &self.source[..frame.cursor.end],
                frame.cursor.position,
            )? {
                let range = frame.cursor.position + body.start..frame.cursor.position + body.end;
                let key_skip = frame
                    .cursor
                    .key_skip
                    .filter(|offset| range.contains(offset));
                let index = frame.meta.conditionals.len();
                let block_index = self.reserve_block();
                frame.meta.conditionals.push((condition, block_index));
                let _ = write!(frame.meta.html, "<!--c:{index}-->");
                frame.cursor.position += consumed;
                return Ok(Some(SectionFrame::new(range, Some(block_index), key_skip)));
            }
        }
        if opening_name == Some("for") {
            if let Some(repeat) = parse_for_block(
                self.component,
                &self.source[..frame.cursor.end],
                frame.cursor.position,
            )? {
                return Ok(Some(self.suspend_repeat(frame, repeat)));
            }
        }
        self.compile_literal(frame)?;
        Ok(None)
    }

    fn reserve_block(&mut self) -> usize {
        let index = self.blocks.len();
        self.blocks.push(TemplateSectionMeta::default());
        index
    }

    fn suspend_repeat(&mut self, frame: &mut SectionFrame, repeat: ParsedForBlock) -> SectionFrame {
        let range =
            frame.cursor.position + repeat.body.start..frame.cursor.position + repeat.body.end;
        let key_skip = repeat.key_skip.map(|offset| frame.cursor.position + offset);
        let block_index = self.reserve_block();
        let index = frame.meta.repeats.len();
        frame.meta.repeats.push(CompiledRepeat {
            collection: repeat.collection,
            item_var: repeat.item_var,
            block_index,
            key_path: repeat.key_path,
        });
        let _ = write!(frame.meta.html, "<!--r:{index}-->");
        frame.cursor.position += repeat.consumed;
        SectionFrame::new(range, Some(block_index), key_skip)
    }

    fn compile_named_directive(&self, frame: &mut SectionFrame) -> bool {
        let Some(declarations) = self.declarations else {
            return false;
        };
        if let Some(declaration) = declarations.declaration_at(frame.cursor.position) {
            frame.cursor.position = declaration.range.end;
            return true;
        }
        let Some(render) = declarations.render_at(frame.cursor.position) else {
            return false;
        };
        let index = frame.meta.renders.len();
        frame.meta.renders.push(CompiledRender {
            block_index: render.target,
            scope: render.scope.clone(),
            alias: render.alias.clone(),
        });
        let _ = write!(frame.meta.html, "<!--u:{index}-->");
        frame.cursor.position = render.range.end;
        true
    }

    fn compile_literal(&self, frame: &mut SectionFrame) -> Result<()> {
        let remaining = &self.source[frame.cursor.position..frame.cursor.end];
        if remaining.starts_with('<') && self.compile_tag(frame)? {
            return Ok(());
        }
        if remaining.starts_with('@') && is_inside_tag(self.source, frame.cursor.position) {
            if let Some((event, handler, args, consumed)) = parse_event_attr(
                self.component,
                &self.source[..frame.cursor.end],
                frame.cursor.position,
            )? {
                frame.meta.events.push((event, handler, args));
                frame.meta.html.push_str("data-ev=\"1\"");
                frame.cursor.position += consumed;
                return Ok(());
            }
        }
        if let Some(ch) = remaining.chars().next() {
            frame.meta.html.push(ch);
            frame.cursor.position += ch.len_utf8();
        }
        Ok(())
    }

    fn compile_tag(&self, frame: &mut SectionFrame) -> Result<bool> {
        let input = &self.source[..frame.cursor.end];
        let remaining = &input[frame.cursor.position..];
        let key_skip = frame
            .cursor
            .key_skip
            .and_then(|offset| offset.checked_sub(frame.cursor.position));
        if self.declarations.is_some() && frame.cursor.position >= frame.cursor.raw_end {
            if let Some(tag) = parse_tag(remaining) {
                if tag.name == "boundary" {
                    frame.cursor.position += tag.close + 1;
                    return Ok(true);
                }
                if !tag.closing && !tag.self_closing && is_raw_text_or_rcdata_element(tag.name) {
                    frame.cursor.raw_end = frame.cursor.position
                        + raw_text_element_end(remaining, tag.name, tag.close + 1);
                }
            }
        }
        if let Some((open_end, close_start, close_end)) =
            find_style_element_bounds(input, frame.cursor.position)
        {
            if let Some((html, _)) =
                parse_regular_tag(self.component, remaining, &mut frame.meta, key_skip)?
            {
                frame.meta.html.push_str(&html);
            }
            compile_style_content(&input[open_end..close_start], &mut frame.meta);
            frame.meta.html.push_str(&input[close_start..close_end]);
            frame.cursor.position = close_end;
            return Ok(true);
        }
        if remaining.starts_with("<outlet") {
            if let Some(consumed) = find_element_end(remaining, "outlet") {
                frame.meta.html.push_str("<outlet></outlet>");
                frame.cursor.position += consumed;
                return Ok(true);
            }
        }
        if let Some((html, consumed)) =
            parse_regular_tag(self.component, remaining, &mut frame.meta, key_skip)?
        {
            frame.meta.html.push_str(&html);
            frame.cursor.position += consumed;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Preserve the existing whitespace normalization without allocating a body.
pub(super) fn trimmed_range(input: &str, range: Range<usize>) -> Range<usize> {
    let body = &input[range.clone()];
    let start = range.start + body.len() - body.trim_start().len();
    start..start + body.trim().len()
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
