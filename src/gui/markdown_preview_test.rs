use super::*;

#[test]
fn render_width_clamps_to_visible_minimum() {
    assert_eq!(render_width(0.0), 80.0);
    assert_eq!(render_width(241.4), 241.0);
    assert_eq!(render_width(241.5), 242.0);
}

#[test]
fn parse_markdown_preserves_nested_list_shape() {
    let blocks = parse_markdown("- 你绝对不会说以下词汇\n  - 收口\n  - 一刀\n- 说话很可爱\n");

    let [MarkdownBlock::List(list)] = blocks.as_slice() else {
        panic!("expected one top-level list");
    };
    assert_eq!(list.items.len(), 2);
    assert!(matches!(
        list.items[0].blocks[0],
        MarkdownBlock::Paragraph(_)
    ));
    let MarkdownBlock::List(nested) = &list.items[0].blocks[1] else {
        panic!("expected nested list");
    };
    assert_eq!(nested.items.len(), 2);
}

#[test]
fn parse_markdown_keeps_inline_code_inside_heading() {
    let blocks = parse_markdown("# `read_view` Working Rules\n");

    let [MarkdownBlock::Heading { spans, .. }] = blocks.as_slice() else {
        panic!("expected heading");
    };
    assert_eq!(spans.len(), 2);
    assert!(spans[0].style.code);
    assert_eq!(spans[0].text, "read_view");
}

#[test]
fn parse_markdown_does_not_treat_shell_paths_as_math_code() {
    let blocks = parse_markdown("@$HOME/.codex/a.md\n@$HOME/.codex/b.md\n");

    let [MarkdownBlock::Paragraph(spans)] = blocks.as_slice() else {
        panic!("expected paragraph");
    };
    assert!(spans.iter().all(|span| !span.style.code));
}

#[test]
fn quote_marker_rect_spans_full_content_height() {
    let content_rect = Rect::from_min_max(Pos2::new(20.0, 10.0), Pos2::new(160.0, 82.0));
    let marker_rect = quote_marker_rect(4.0, content_rect);

    assert_eq!(marker_rect.left(), 4.0);
    assert_eq!(marker_rect.right(), 8.0);
    assert_eq!(marker_rect.top(), content_rect.top());
    assert_eq!(marker_rect.bottom(), content_rect.bottom());
}

#[test]
fn parse_paragraph_preserves_soft_line_breaks() {
    let blocks = parse_markdown(
        "Phase: 03 (overview-to-reviewer-integration) -- VERIFYING\nPlan: 1 of 1\n**Status:** Phase complete\n",
    );

    let [MarkdownBlock::Paragraph(spans)] = blocks.as_slice() else {
        panic!("expected one paragraph");
    };
    let text = spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>();
    assert!(text.contains("VERIFYING\nPlan: 1 of 1\nStatus:"));
}

#[test]
fn parse_inline_styles_preserves_commonmark_emphasis_semantics() {
    let blocks = parse_markdown("**strong** *em* ~~gone~~ `code` [link](https://example.com)\n");

    let [MarkdownBlock::Paragraph(spans)] = blocks.as_slice() else {
        panic!("expected one paragraph");
    };

    let strong = spans.iter().find(|span| span.text == "strong").unwrap();
    let emphasis = spans.iter().find(|span| span.text == "em").unwrap();
    let strike = spans.iter().find(|span| span.text == "gone").unwrap();
    let code = spans.iter().find(|span| span.text == "code").unwrap();
    let link = spans.iter().find(|span| span.text == "link").unwrap();

    assert!(strong.style.strong);
    assert!(emphasis.style.emphasis);
    assert!(strike.style.strikethrough);
    assert!(code.style.code);
    assert_eq!(link.style.link.as_deref(), Some("https://example.com"));
}

#[test]
fn inline_layout_uses_registered_strong_font_family_for_strong_spans() {
    let ctx = egui::Context::default();
    theme::configure(&ctx);

    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let spans = vec![InlineSpan {
                text: "strong".to_string(),
                style: InlineStyle {
                    strong: true,
                    ..Default::default()
                },
            }];

            let job = inline_layout_job(
                &spans,
                200.0,
                14.0,
                false,
                Align::LEFT,
                theme::ThemeMode::Dark,
                ui,
            );

            assert_eq!(
                job.sections[0].format.font_id.family,
                theme::markdown_strong_font_family()
            );
        });
    });
}

#[test]
fn inline_layout_makes_strong_and_code_visible_across_themes() {
    let ctx = egui::Context::default();
    theme::configure(&ctx);

    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for mode in [theme::ThemeMode::Light, theme::ThemeMode::Dark] {
                let spans = vec![
                    InlineSpan {
                        text: "strong".to_string(),
                        style: InlineStyle {
                            strong: true,
                            ..Default::default()
                        },
                    },
                    InlineSpan {
                        text: "code".to_string(),
                        style: InlineStyle {
                            code: true,
                            ..Default::default()
                        },
                    },
                ];

                let job = inline_layout_job(&spans, 200.0, 14.0, false, Align::LEFT, mode, ui);
                assert_ne!(job.sections[0].format.color, theme::markdown_text_for(mode));
                assert_ne!(job.sections[1].format.color, theme::markdown_text_for(mode));
                assert_ne!(
                    job.sections[1].format.background,
                    ui.visuals().code_bg_color
                );
            }
        });
    });
}

#[test]
fn parse_task_list_marker_stays_on_list_item() {
    let blocks = parse_markdown("- [x] **OVRV-01**: User can run `gsdv`.\n");

    let [MarkdownBlock::List(list)] = blocks.as_slice() else {
        panic!("expected task list");
    };
    assert_eq!(list.items[0].checked, Some(true));
    let [MarkdownBlock::Paragraph(spans)] = list.items[0].blocks.as_slice() else {
        panic!("expected a single paragraph inside the task item");
    };
    assert_eq!(
        spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>(),
        "OVRV-01: User can run gsdv."
    );
    assert!(
        spans
            .iter()
            .any(|span| span.style.code && span.text == "gsdv")
    );
}

#[test]
fn parse_yaml_front_matter_as_code_block() {
    let blocks =
        parse_markdown("---\ngsd_state_version: 1.0\nprogress:\n  percent: 100\n---\n# Body\n");

    let [
        MarkdownBlock::CodeBlock { language, text },
        MarkdownBlock::Heading { spans, .. },
    ] = blocks.as_slice()
    else {
        panic!("expected front matter code block followed by heading");
    };
    assert_eq!(language.as_deref(), Some("yaml"));
    assert!(text.contains("progress:\n  percent: 100"));
    assert_eq!(spans[0].text, "Body");
}

#[test]
fn parse_footnote_definition_as_footnote_block() {
    let blocks = parse_markdown("Body with footnote.[^1]\n\n[^1]: Footnote text\n");

    assert!(matches!(blocks[0], MarkdownBlock::Paragraph(_)));
    let MarkdownBlock::FootnoteDefinition { label, blocks } = &blocks[1] else {
        panic!("expected footnote definition block");
    };
    assert_eq!(label, "1");
    let [MarkdownBlock::Paragraph(spans)] = blocks.as_slice() else {
        panic!("expected footnote paragraph");
    };
    assert_eq!(spans[0].text, "Footnote text");
}

#[test]
fn parse_pipe_table_as_table_block() {
    let blocks = parse_markdown(
        "| Feature | Reason |\n| --- | --- |\n| Custom diff engine | Git is sufficient |\n",
    );

    let [MarkdownBlock::Table(table)] = blocks.as_slice() else {
        panic!("expected table block");
    };
    assert_eq!(
        table.alignments,
        vec![TableAlignment::None, TableAlignment::None]
    );
    assert_eq!(table.headers.len(), 2);
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.headers[0][0].text, "Feature");
    assert_eq!(table.rows[0][1][0].text, "Git is sufficient");
}

#[test]
fn parse_pipe_table_preserves_header_width_and_alignment() {
    let blocks =
        parse_markdown("| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c | d |\n");

    let [MarkdownBlock::Table(table)] = blocks.as_slice() else {
        panic!("expected table block");
    };
    assert_eq!(
        table.alignments,
        vec![
            TableAlignment::Left,
            TableAlignment::Center,
            TableAlignment::Right
        ]
    );
    assert_eq!(table.headers.len(), 3);
    assert_eq!(table.rows[0].len(), 3);
    assert_eq!(table.rows[0][2][0].text, "c");
}

#[test]
fn table_column_widths_fill_available_width() {
    let widths = table_column_widths(1000.0, 3);

    assert_eq!(widths.len(), 3);
    assert_eq!(widths.iter().sum::<f32>(), 1000.0);
    assert!(widths.iter().all(|width| *width >= 72.0));
}
