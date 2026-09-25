// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{compile_to_metadata, generate_compiled_template_with_root_source, Result};

const IF_CONTROL: &str =
    "<article><if condition=\"enabled\"><button>{{label}}</button></if></article>";

fn assert_same_metadata(source: &str, control: &str) -> Result<()> {
    let compile = |html| {
        generate_compiled_template_with_root_source("test-whitespace", html, html, false, false)
    };
    let actual = compile(source)?;
    let expected = compile(control)?;
    assert_eq!(
        actual.template_json, expected.template_json,
        "source bytes: {source:?}",
    );
    assert_eq!(
        actual.template_functions, expected.template_functions,
        "source bytes: {source:?}",
    );
    Ok(())
}

#[test]
fn lf_opening_control() -> Result<()> {
    assert_same_metadata(
        "<article><if\n condition=\"enabled\"\n><button>{{label}}</button></if></article>",
        IF_CONTROL,
    )
}

#[test]
fn crlf_opening_directive() -> Result<()> {
    assert_same_metadata(
        "<article><if\r\n condition=\"enabled\"\r\n><button>{{label}}</button></if></article>",
        IF_CONTROL,
    )
}

#[test]
fn lf_closing_directive() -> Result<()> {
    assert_same_metadata(
        "<article><if condition=\"enabled\"><button>{{label}}</button></if\n></article>",
        IF_CONTROL,
    )
}

#[test]
fn space_closing_directive() -> Result<()> {
    assert_same_metadata(
        "<article><if condition=\"enabled\"><button>{{label}}</button></if ></article>",
        IF_CONTROL,
    )
}

#[test]
fn nested_closing_directive_preserves_footer_owner() -> Result<()> {
    assert_same_metadata(
        "<article><if condition=\"enabled\"><footer><if condition=\"ready\"><button>{{label}}</button></if\n></footer></if></article>",
        "<article><if condition=\"enabled\"><footer><if condition=\"ready\"><button>{{label}}</button></if></footer></if></article>",
    )
}

#[test]
fn crlf_between_complete_tags_control() -> Result<()> {
    let source =
        "<article>\r\n<if condition=\"enabled\"><button>{{label}}</button></if>\r\n</article>";
    let meta = compile_to_metadata("test-whitespace", source.into(), Vec::new())?;
    assert_eq!(meta.root.html, "<article>\r\n\r\n</article>");
    assert_eq!(meta.root.conditionals.len(), 1);
    assert_eq!(meta.blocks.len(), 1);
    assert_eq!(meta.blocks[0].html, "<button></button>");
    Ok(())
}

#[test]
fn all_html_whitespace_in_opening_and_closing_directives() -> Result<()> {
    for (name, attribute) in [
        ("if", "condition=\"enabled\""),
        ("for", "each=\"item in items\""),
    ] {
        let control = format!(
            "<article><{name} {attribute}><button>{{{{label}}}}</button></{name}></article>"
        );
        for whitespace in [" ", "\t", "\n", "\r", "\r\n", "\x0c"] {
            let opening = format!(
                "<article><{name}{whitespace}{attribute}{whitespace}><button>{{{{label}}}}</button></{name}></article>"
            );
            assert_same_metadata(&opening, &control)?;
            let closing = format!(
                "<article><{name} {attribute}><button>{{{{label}}}}</button></{name}{whitespace}></article>"
            );
            assert_same_metadata(&closing, &control)?;
        }
    }
    Ok(())
}

#[test]
fn nested_adjacent_keyed_scopes_preserve_all_metadata() -> Result<()> {
    let control = concat!(
        "<article><if condition=\"enabled\"><footer>",
        "<if condition=\"ready\"><button title=\"{{label}}\" @click=\"{select(label)}\">{{label}}</button></if>",
        "<if condition=\"other\"><input :value=\"{{label}}\"></if>",
        "<for each=\"group in groups\"><section key=\"{{group.id}}\" data-id=\"{{group.id}}\">",
        "<for each=\"item in group.items\"><if condition=\"item.ready\">",
        "<button key=\"{{item.id}}\" ?disabled=\"{{item.disabled}}\" @click=\"{select(item.id)}\">{{item.label}}</button>",
        "</if></for></section></for>",
        "<for each=\"item in items\"><span key=\"{{item.id}}\">{{item.label}}</span></for>",
        "</footer></if><p title=\"{{label}}\">{{label}}</p></article>",
    );
    for whitespace in [" ", "\t", "\n", "\r", "\r\n", "\x0c"] {
        let opening = control
            .replace("<if ", &format!("<if{whitespace}"))
            .replace("<for ", &format!("<for{whitespace}"));
        assert_same_metadata(&opening, control)?;
        let closing = control
            .replace("</if>", &format!("</if{whitespace}>"))
            .replace("</for>", &format!("</for{whitespace}>"));
        assert_same_metadata(&closing, control)?;
        let both = opening
            .replace("</if>", &format!("</if{whitespace}>"))
            .replace("</for>", &format!("</for{whitespace}>"));
        assert_same_metadata(&both, control)?;
    }
    Ok(())
}

#[test]
fn directive_attributes_use_html_scanner() -> Result<()> {
    assert_same_metadata(
        "<article><if\tcondition \r\n= 'enabled'><button>{{label}}</button></if \n></article>",
        IF_CONTROL,
    )?;
    assert_same_metadata(
        "<article><if condition=enabled><button>{{label}}</button></if\t></article>",
        IF_CONTROL,
    )?;
    assert_same_metadata(
        "<for\teach \n= 'item in items'><span>{{item.label}}</span></for\r\n>",
        "<for each=\"item in items\"><span>{{item.label}}</span></for>",
    )?;
    assert_same_metadata(
        "<if data-condition=\"ignored\" condition = 'count > 0'><p>yes</p></if\n>",
        "<if condition=\"count > 0\"><p>yes</p></if>",
    )
}

#[test]
fn matching_ignores_directive_text_in_attributes_and_comments() -> Result<()> {
    let control = concat!(
        "<article><if condition=\"enabled\" data-end=\"</if>\">",
        "<!-- <if condition='fake'> </if> <for each='x in xs'> -->",
        "<p title=\"<if fake> </if> <for fake> </for>\">&lt;if&gt; native text</p>",
        "<iframe title=\"native\"></iframe><if-example>literal</if-example>",
        "<for each=\"item in items\" data-open=\"<for fake>\">",
        "<span title=\"</for>\">{{item.label}}</span></for>",
        "</if></article>",
    );
    let source = control
        .replacen("<if condition", "<if\r\ncondition", 1)
        .replace("</if></article>", "</if \n></article>")
        .replace("</span></for>", "</span></for\t>");
    assert_same_metadata(&source, control)?;
    let meta = compile_to_metadata("test-whitespace", source.as_str().into(), Vec::new())?;
    assert_eq!(meta.root.conditionals.len(), 1);
    assert_eq!(meta.blocks.len(), 2);
    assert!(meta.blocks[0].html.contains("&lt;if&gt; native text"));
    assert!(meta.blocks[0]
        .html
        .contains("<iframe title=\"native\"></iframe>"));
    assert!(meta.blocks[0]
        .html
        .contains("<if-example>literal</if-example>"));
    assert!(meta.blocks[0]
        .html
        .contains("<if fake> </if> <for fake> </for>"));
    Ok(())
}

#[test]
fn closing_names_follow_html_matching() -> Result<()> {
    assert_same_metadata(
        "<article><if condition=\"enabled\"><button>{{label}}</button></IF \n></article>",
        IF_CONTROL,
    )
}
