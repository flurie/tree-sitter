use super::helpers::fixtures::{get_language, get_test_language};
use crate::generate::generate_parser_for_grammar;
use std::{thread, usize};
use tree_sitter::{InputEdit, LogType, Parser, Point, Range};

#[test]
fn test_basic_parsing() {
    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();

    let tree = parser
        .parse_str(
            "
        struct Stuff {}
        fn main() {}
    ",
            None,
        )
        .unwrap();

    let root_node = tree.root_node();
    assert_eq!(root_node.kind(), "source_file");

    assert_eq!(
        root_node.to_sexp(),
        "(source_file (struct_item (type_identifier) (field_declaration_list)) (function_item (identifier) (parameters) (block)))"
    );

    let struct_node = root_node.child(0).unwrap();
    assert_eq!(struct_node.kind(), "struct_item");
}

#[test]
fn test_parsing_with_logging() {
    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();

    let mut messages = Vec::new();
    parser.set_logger(Some(Box::new(|log_type, message| {
        messages.push((log_type, message.to_string()));
    })));

    parser
        .parse_str(
            "
        struct Stuff {}
        fn main() {}
    ",
            None,
        )
        .unwrap();

    assert!(messages.contains(&(
        LogType::Parse,
        "reduce sym:struct_item, child_count:3".to_string()
    )));
    assert!(messages.contains(&(LogType::Lex, "skip character:' '".to_string())));
}

#[test]
fn test_parsing_with_custom_utf8_input() {
    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();

    let lines = &["pub fn foo() {", "  1", "}"];

    let tree = parser
        .parse_utf8(
            &mut |_, position| {
                let row = position.row;
                let column = position.column;
                if row < lines.len() {
                    if column < lines[row].as_bytes().len() {
                        &lines[row].as_bytes()[column..]
                    } else {
                        "\n".as_bytes()
                    }
                } else {
                    &[]
                }
            },
            None,
        )
        .unwrap();

    let root = tree.root_node();
    assert_eq!(root.to_sexp(), "(source_file (function_item (visibility_modifier) (identifier) (parameters) (block (integer_literal))))");
    assert_eq!(root.kind(), "source_file");
    assert_eq!(root.has_error(), false);
    assert_eq!(root.child(0).unwrap().kind(), "function_item");
}

#[test]
fn test_parsing_with_custom_utf16_input() {
    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();

    let lines: Vec<Vec<u16>> = ["pub fn foo() {", "  1", "}"]
        .iter()
        .map(|s| s.encode_utf16().collect())
        .collect();

    let tree = parser
        .parse_utf16(
            &mut |_, position| {
                let row = position.row;
                let column = position.column;
                if row < lines.len() {
                    if column < lines[row].len() {
                        &lines[row][column..]
                    } else {
                        &[10]
                    }
                } else {
                    &[]
                }
            },
            None,
        )
        .unwrap();

    let root = tree.root_node();
    assert_eq!(root.to_sexp(), "(source_file (function_item (visibility_modifier) (identifier) (parameters) (block (integer_literal))))");
    assert_eq!(root.kind(), "source_file");
    assert_eq!(root.has_error(), false);
    assert_eq!(root.child(0).unwrap().kind(), "function_item");
}

#[test]
fn test_parsing_after_editing() {
    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();

    let mut input_bytes = "fn test(a: A, c: C) {}".as_bytes();
    let mut input_bytes_read = Vec::new();

    let mut tree = parser
        .parse_utf8(
            &mut |offset, _| {
                let offset = offset;
                if offset < input_bytes.len() {
                    let result = &input_bytes[offset..offset + 1];
                    input_bytes_read.extend(result.iter());
                    result
                } else {
                    &[]
                }
            },
            None,
        )
        .unwrap();

    let parameters_sexp = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(1)
        .unwrap()
        .to_sexp();
    assert_eq!(
        parameters_sexp,
        "(parameters (parameter (identifier) (type_identifier)) (parameter (identifier) (type_identifier)))"
    );

    input_bytes_read.clear();
    input_bytes = "fn test(a: A, b: B, c: C) {}".as_bytes();
    tree.edit(&InputEdit {
        start_byte: 14,
        old_end_byte: 14,
        new_end_byte: 20,
        start_position: Point::new(0, 14),
        old_end_position: Point::new(0, 14),
        new_end_position: Point::new(0, 20),
    });

    let tree = parser
        .parse_utf8(
            &mut |offset, _| {
                let offset = offset;
                if offset < input_bytes.len() {
                    let result = &input_bytes[offset..offset + 1];
                    input_bytes_read.extend(result.iter());
                    result
                } else {
                    &[]
                }
            },
            Some(&tree),
        )
        .unwrap();

    let parameters_sexp = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(1)
        .unwrap()
        .to_sexp();
    assert_eq!(
        parameters_sexp,
        "(parameters (parameter (identifier) (type_identifier)) (parameter (identifier) (type_identifier)) (parameter (identifier) (type_identifier)))"
    );

    let retokenized_content = String::from_utf8(input_bytes_read).unwrap();
    assert!(retokenized_content.contains("b: B"));
    assert!(!retokenized_content.contains("a: A"));
    assert!(!retokenized_content.contains("c: C"));
    assert!(!retokenized_content.contains("{}"));
}

#[test]
fn test_parsing_on_multiple_threads() {
    // Parse this source file so that each thread has a non-trivial amount of
    // work to do.
    let this_file_source = include_str!("parser_test.rs");

    let mut parser = Parser::new();
    parser.set_language(get_language("rust")).unwrap();
    let tree = parser.parse_str(this_file_source, None).unwrap();

    let mut parse_threads = Vec::new();
    for thread_id in 1..5 {
        let mut tree_clone = tree.clone();
        parse_threads.push(thread::spawn(move || {
            // For each thread, prepend a different number of declarations to the
            // source code.
            let mut prepend_line_count = 0;
            let mut prepended_source = String::new();
            for _ in 0..thread_id {
                prepend_line_count += 2;
                prepended_source += "struct X {}\n\n";
            }

            tree_clone.edit(&InputEdit {
                start_byte: 0,
                old_end_byte: 0,
                new_end_byte: prepended_source.len(),
                start_position: Point::new(0, 0),
                old_end_position: Point::new(0, 0),
                new_end_position: Point::new(prepend_line_count, 0),
            });
            prepended_source += this_file_source;

            // Reparse using the old tree as a starting point.
            let mut parser = Parser::new();
            parser.set_language(get_language("rust")).unwrap();
            parser
                .parse_str(&prepended_source, Some(&tree_clone))
                .unwrap()
        }));
    }

    // Check that the trees have the expected relationship to one another.
    let trees = parse_threads
        .into_iter()
        .map(|thread| thread.join().unwrap());
    let child_count_differences = trees
        .map(|t| t.root_node().child_count() - tree.root_node().child_count())
        .collect::<Vec<_>>();

    assert_eq!(child_count_differences, &[1, 2, 3, 4]);
}

// Operation limits

#[test]
fn test_parsing_with_an_operation_limit() {
    let mut parser = Parser::new();
    parser.set_language(get_language("json")).unwrap();

    // Start parsing from an infinite input. Parsing should abort after 5 "operations".
    parser.set_operation_limit(5);
    let mut call_count = 0;
    let tree = parser.parse_utf8(
        &mut |_, _| {
            if call_count == 0 {
                call_count += 1;
                b"[0"
            } else {
                call_count += 1;
                b", 0"
            }
        },
        None,
    );
    assert!(tree.is_none());
    assert!(call_count >= 3);
    assert!(call_count <= 8);

    // Resume parsing from the previous state.
    call_count = 0;
    parser.set_operation_limit(20);
    let tree = parser
        .parse_utf8(
            &mut |_, _| {
                if call_count == 0 {
                    call_count += 1;
                    b"]"
                } else {
                    b""
                }
            },
            None,
        )
        .unwrap();
    assert_eq!(
        tree.root_node().to_sexp(),
        "(value (array (number) (number) (number)))"
    );
}

#[test]
fn test_parsing_with_a_reset_after_reaching_an_operation_limit() {
    let mut parser = Parser::new();
    parser.set_language(get_language("json")).unwrap();

    parser.set_operation_limit(3);
    let tree = parser.parse_str("[1234, 5, 6, 7, 8]", None);
    assert!(tree.is_none());

    // Without calling reset, the parser continues from where it left off, so
    // it does not see the changes to the beginning of the source code.
    parser.set_operation_limit(usize::MAX);
    let tree = parser.parse_str("[null, 5, 6, 4, 5]", None).unwrap();
    assert_eq!(
        tree.root_node().to_sexp(),
        "(value (array (number) (number) (number) (number) (number)))"
    );

    parser.set_operation_limit(3);
    let tree = parser.parse_str("[1234, 5, 6, 7, 8]", None);
    assert!(tree.is_none());

    // By calling reset, we force the parser to start over from scratch so
    // that it sees the changes to the beginning of the source code.
    parser.set_operation_limit(usize::MAX);
    parser.reset();
    let tree = parser.parse_str("[null, 5, 6, 4, 5]", None).unwrap();
    assert_eq!(
        tree.root_node().to_sexp(),
        "(value (array (null) (number) (number) (number) (number)))"
    );
}

// Included Ranges

#[test]
fn test_parsing_with_one_included_range() {
    let source_code = "<span>hi</span><script>console.log('sup');</script>";

    let mut parser = Parser::new();
    parser.set_language(get_language("html")).unwrap();
    let html_tree = parser.parse_str(source_code, None).unwrap();
    let script_content_node = html_tree.root_node().child(1).unwrap().child(1).unwrap();
    assert_eq!(script_content_node.kind(), "raw_text");

    parser.set_included_ranges(&[script_content_node.range()]);
    parser.set_language(get_language("javascript")).unwrap();
    let js_tree = parser.parse_str(source_code, None).unwrap();

    assert_eq!(
        js_tree.root_node().to_sexp(),
        concat!(
            "(program (expression_statement (call_expression",
            " (member_expression (identifier) (property_identifier))",
            " (arguments (string)))))",
        )
    );
    assert_eq!(
        js_tree.root_node().start_position(),
        Point::new(0, source_code.find("console").unwrap())
    );
}

#[test]
fn test_parsing_with_multiple_included_ranges() {
    let source_code = "html `<div>Hello, ${name.toUpperCase()}, it's <b>${now()}</b>.</div>`";

    let mut parser = Parser::new();
    parser.set_language(get_language("javascript")).unwrap();
    let js_tree = parser.parse_str(source_code, None).unwrap();
    let template_string_node = js_tree
        .root_node()
        .descendant_for_byte_range(
            source_code.find("<div>").unwrap(),
            source_code.find("Hello").unwrap(),
        )
        .unwrap();
    assert_eq!(template_string_node.kind(), "template_string");

    let open_quote_node = template_string_node.child(0).unwrap();
    let interpolation_node1 = template_string_node.child(1).unwrap();
    let interpolation_node2 = template_string_node.child(2).unwrap();
    let close_quote_node = template_string_node.child(3).unwrap();

    parser.set_language(get_language("html")).unwrap();
    parser.set_included_ranges(&[
        Range {
            start_byte: open_quote_node.end_byte(),
            start_point: open_quote_node.end_position(),
            end_byte: interpolation_node1.start_byte(),
            end_point: interpolation_node1.start_position(),
        },
        Range {
            start_byte: interpolation_node1.end_byte(),
            start_point: interpolation_node1.end_position(),
            end_byte: interpolation_node2.start_byte(),
            end_point: interpolation_node2.start_position(),
        },
        Range {
            start_byte: interpolation_node2.end_byte(),
            start_point: interpolation_node2.end_position(),
            end_byte: close_quote_node.start_byte(),
            end_point: close_quote_node.start_position(),
        },
    ]);
    let html_tree = parser.parse_str(source_code, None).unwrap();

    assert_eq!(
        html_tree.root_node().to_sexp(),
        concat!(
            "(fragment (element",
            " (start_tag (tag_name))",
            " (text)",
            " (element (start_tag (tag_name)) (end_tag (tag_name)))",
            " (text)",
            " (end_tag (tag_name))))",
        )
    );

    let div_element_node = html_tree.root_node().child(0).unwrap();
    let hello_text_node = div_element_node.child(1).unwrap();
    let b_element_node = div_element_node.child(2).unwrap();
    let b_start_tag_node = b_element_node.child(0).unwrap();
    let b_end_tag_node = b_element_node.child(1).unwrap();

    assert_eq!(hello_text_node.kind(), "text");
    assert_eq!(
        hello_text_node.start_byte(),
        source_code.find("Hello").unwrap()
    );
    assert_eq!(hello_text_node.end_byte(), source_code.find("<b>").unwrap());

    assert_eq!(b_start_tag_node.kind(), "start_tag");
    assert_eq!(
        b_start_tag_node.start_byte(),
        source_code.find("<b>").unwrap()
    );
    assert_eq!(
        b_start_tag_node.end_byte(),
        source_code.find("${now()}").unwrap()
    );

    assert_eq!(b_end_tag_node.kind(), "end_tag");
    assert_eq!(
        b_end_tag_node.start_byte(),
        source_code.find("</b>").unwrap()
    );
    assert_eq!(
        b_end_tag_node.end_byte(),
        source_code.find(".</div>").unwrap()
    );
}

#[test]
fn test_parsing_utf16_code_with_errors_at_the_end_of_an_included_range() {
    let source_code = "<script>a.</script>";
    let utf16_source_code: Vec<u16> = source_code.as_bytes().iter().map(|c| *c as u16).collect();

    let start_byte = 2 * source_code.find("a.").unwrap();
    let end_byte = 2 * source_code.find("</script>").unwrap();

    let mut parser = Parser::new();
    parser.set_language(get_language("javascript")).unwrap();
    parser.set_included_ranges(&[Range {
        start_byte,
        end_byte,
        start_point: Point::new(0, start_byte),
        end_point: Point::new(0, end_byte),
    }]);
    let tree = parser
        .parse_utf16(&mut |i, _| &utf16_source_code[i..], None)
        .unwrap();
    assert_eq!(tree.root_node().to_sexp(), "(program (ERROR (identifier)))");
}

#[test]
fn test_parsing_with_external_scanner_that_uses_included_range_boundaries() {
    let source_code = "a <%= b() %> c <% d() %>";
    let range1_start_byte = source_code.find(" b() ").unwrap();
    let range1_end_byte = range1_start_byte + " b() ".len();
    let range2_start_byte = source_code.find(" d() ").unwrap();
    let range2_end_byte = range2_start_byte + " d() ".len();

    let mut parser = Parser::new();
    parser.set_language(get_language("javascript")).unwrap();
    parser.set_included_ranges(&[
        Range {
            start_byte: range1_start_byte,
            end_byte: range1_end_byte,
            start_point: Point::new(0, range1_start_byte),
            end_point: Point::new(0, range1_end_byte),
        },
        Range {
            start_byte: range2_start_byte,
            end_byte: range2_end_byte,
            start_point: Point::new(0, range2_start_byte),
            end_point: Point::new(0, range2_end_byte),
        },
    ]);

    let tree = parser.parse_str(source_code, None).unwrap();
    let root = tree.root_node();
    let statement1 = root.child(0).unwrap();
    let statement2 = root.child(1).unwrap();

    assert_eq!(
        root.to_sexp(),
        concat!(
            "(program",
            " (expression_statement (call_expression (identifier) (arguments)))",
            " (expression_statement (call_expression (identifier) (arguments))))"
        )
    );

    assert_eq!(statement1.start_byte(), source_code.find("b()").unwrap());
    assert_eq!(statement1.end_byte(), source_code.find(" %> c").unwrap());
    assert_eq!(statement2.start_byte(), source_code.find("d()").unwrap());
    assert_eq!(statement2.end_byte(), source_code.len() - " %>".len());
}

#[test]
fn test_parsing_with_a_newly_excluded_range() {
    let mut source_code = String::from("<div><span><%= something %></span></div>");

    // Parse HTML including the template directive, which will cause an error
    let mut parser = Parser::new();
    parser.set_language(get_language("html")).unwrap();
    let mut first_tree = parser.parse_str(&source_code, None).unwrap();

    // Insert code at the beginning of the document.
    let prefix = "a very very long line of plain text. ";
    first_tree.edit(&InputEdit {
        start_byte: 0,
        old_end_byte: 0,
        new_end_byte: prefix.len(),
        start_position: Point::new(0, 0),
        old_end_position: Point::new(0, 0),
        new_end_position: Point::new(0, prefix.len()),
    });
    source_code.insert_str(0, prefix);

    // Parse the HTML again, this time *excluding* the template directive
    // (which has moved since the previous parse).
    let directive_start = source_code.find("<%=").unwrap();
    let directive_end = source_code.find("</span>").unwrap();
    let source_code_end = source_code.len();
    parser.set_included_ranges(&[
        Range {
            start_byte: 0,
            end_byte: directive_start,
            start_point: Point::new(0, 0),
            end_point: Point::new(0, directive_start),
        },
        Range {
            start_byte: directive_end,
            end_byte: source_code_end,
            start_point: Point::new(0, directive_end),
            end_point: Point::new(0, source_code_end),
        },
    ]);
    let tree = parser.parse_str(&source_code, Some(&first_tree)).unwrap();

    assert_eq!(
        tree.root_node().to_sexp(),
        concat!(
            "(fragment (text) (element",
            " (start_tag (tag_name))",
            " (element (start_tag (tag_name)) (end_tag (tag_name)))",
            " (end_tag (tag_name))))"
        )
    );

    assert_eq!(
        tree.changed_ranges(&first_tree),
        vec![
            // The first range that has changed syntax is the range of the newly-inserted text.
            Range {
                start_byte: 0,
                end_byte: prefix.len(),
                start_point: Point::new(0, 0),
                end_point: Point::new(0, prefix.len()),
            },
            // Even though no edits were applied to the outer `div` element,
            // its contents have changed syntax because a range of text that
            // was previously included is now excluded.
            Range {
                start_byte: directive_start,
                end_byte: directive_end,
                start_point: Point::new(0, directive_start),
                end_point: Point::new(0, directive_end),
            },
        ]
    );
}

#[test]
fn test_parsing_with_a_newly_included_range() {
    let source_code = "<div><%= foo() %></div><div><%= bar() %>";
    let first_code_start_index = source_code.find(" foo").unwrap();
    let first_code_end_index = first_code_start_index + 7;
    let second_code_start_index = source_code.find(" bar").unwrap();
    let second_code_end_index = second_code_start_index + 7;
    let ranges = [
        Range {
            start_byte: first_code_start_index,
            end_byte: first_code_end_index,
            start_point: Point::new(0, first_code_start_index),
            end_point: Point::new(0, first_code_end_index),
        },
        Range {
            start_byte: second_code_start_index,
            end_byte: second_code_end_index,
            start_point: Point::new(0, second_code_start_index),
            end_point: Point::new(0, second_code_end_index),
        },
    ];

    // Parse only the first code directive as JavaScript
    let mut parser = Parser::new();
    parser.set_language(get_language("javascript")).unwrap();
    parser.set_included_ranges(&ranges[0..1]);
    let first_tree = parser.parse_str(source_code, None).unwrap();
    assert_eq!(
        first_tree.root_node().to_sexp(),
        concat!(
            "(program",
            " (expression_statement (call_expression (identifier) (arguments))))",
        )
    );

    // Parse both the code directives as JavaScript, using the old tree as a reference.
    parser.set_included_ranges(&ranges);
    let tree = parser.parse_str(&source_code, Some(&first_tree)).unwrap();
    assert_eq!(
        tree.root_node().to_sexp(),
        concat!(
            "(program",
            " (expression_statement (call_expression (identifier) (arguments)))",
            " (expression_statement (call_expression (identifier) (arguments))))",
        )
    );

    assert_eq!(
        tree.changed_ranges(&first_tree),
        vec![Range {
            start_byte: first_code_end_index + 1,
            end_byte: second_code_end_index + 1,
            start_point: Point::new(0, first_code_end_index + 1),
            end_point: Point::new(0, second_code_end_index + 1),
        }]
    );
}

#[test]
fn test_parsing_with_included_ranges_and_missing_tokens() {
    let (parser_name, parser_code) = generate_parser_for_grammar(
        r#"{
            "name": "test_leading_missing_token",
            "rules": {
                "program": {
                    "type": "SEQ",
                    "members": [
                        {"type": "SYMBOL", "name": "A"},
                        {"type": "SYMBOL", "name": "b"},
                        {"type": "SYMBOL", "name": "c"},
                        {"type": "SYMBOL", "name": "A"},
                        {"type": "SYMBOL", "name": "b"},
                        {"type": "SYMBOL", "name": "c"}
                    ]
                },
                "A": {"type": "SYMBOL", "name": "a"},
                "a": {"type": "STRING", "value": "a"},
                "b": {"type": "STRING", "value": "b"},
                "c": {"type": "STRING", "value": "c"}
            }
        }"#,
    )
    .unwrap();

    let mut parser = Parser::new();
    parser
        .set_language(get_test_language(&parser_name, &parser_code, None))
        .unwrap();

    // There's a missing `a` token at the beginning of the code. It must be inserted
    // at the beginning of the first included range, not at {0, 0}.
    let source_code = "__bc__bc__";
    parser.set_included_ranges(&[
        Range {
            start_byte: 2,
            end_byte: 4,
            start_point: Point::new(0, 2),
            end_point: Point::new(0, 4),
        },
        Range {
            start_byte: 6,
            end_byte: 8,
            start_point: Point::new(0, 6),
            end_point: Point::new(0, 8),
        },
    ]);

    let tree = parser.parse_str(source_code, None).unwrap();
    let root = tree.root_node();
    assert_eq!(
        root.to_sexp(),
        "(program (A (MISSING)) (b) (c) (A (MISSING)) (b) (c))"
    );
    assert_eq!(root.start_byte(), 2);
    assert_eq!(root.child(3).unwrap().start_byte(), 4);
}
