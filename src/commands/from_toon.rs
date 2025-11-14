use crate::ToonPlugin;
use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{
    record, Category, Example, LabeledError, PipelineData, Signature, SyntaxShape, Value,
};
use toon_format::{decode, types::PathExpansionMode, DecodeOptions, Delimiter, Indent};

pub struct FromToon;

impl SimplePluginCommand for FromToon {
    type Plugin = ToonPlugin;

    fn name(&self) -> &str {
        "from toon"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                "delimiter",
                SyntaxShape::String,
                "Delimiter to use: ',', '|', or \"\\t\" (default is ',')",
                Some('d'),
            )
            .named("expand-paths",
                SyntaxShape::String,
                "Enable path expansion 'off' or 'safe' (defaults to 'off'). When set to 'Safe', dotted keys will be expanded into nested objects if all segments are IdentifierSegments",
                Some('e'),
            )
            .switch("strict", "Enable or disable strict mode (validates array lengths, indentation, etc) (default: true)", Some('s'))
            .named(
                "indent-spaces",
                SyntaxShape::Int,
                "The number of spaces to indent (default is 2)",
                Some('i'),
            )
            .switch("coerce-types", "Enable or disable type coercion (strings like “123” -> numbers) (default: true)", Some('c'))
            .category(Category::Experimental)
    }

    fn description(&self) -> &str {
        "Convert toon formatted text to nushell values"
    }

    fn extra_description(&self) -> &str {
        "'Under the hood' `from toon` calls the `from json` command after decoding the toon formatted input into JSON. Parsing the toon format is set to strict. Make sure the toon output has 2 spaces, is delimited by commas, and uses \\n as a line separator and not \\r\\n even on Windows"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description:
                    "Convert `ls` output to toon format and round trip back to nushell values",
                example: "ls | to toon | from toon",
                result: None,
            },
            Example {
                description: "Convert toon formatted text to nushell values",
                example: r#""[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe\n" | from toon"#,
                result: Some(Value::test_list(vec![
                    Value::test_record(record! {
                        "col1" => Value::test_string("moe".to_string()),
                        "col2" => Value::test_string("larry".to_string()),
                        "col3" => Value::test_string("curly".to_string()),
                    }),
                    Value::test_record(record! {
                        "col1" => Value::test_string("larry".to_string()),
                        "col2" => Value::test_string("curly".to_string()),
                        "col3" => Value::test_string("moe".to_string()),
                    }),
                ])),
            },
        ]
    }

    fn run(
        &self,
        _plugin: &ToonPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: &Value,
    ) -> Result<Value, LabeledError> {
        let delimiter = if let Some(del) = call.get_flag::<String>("delimiter")? {
            match del.as_str() {
                "," => Delimiter::Comma,
                "|" => Delimiter::Pipe,
                "\t" => Delimiter::Tab,
                other => {
                    return Err(
                        LabeledError::new("Invalid Delimiter".to_string()).with_label(
                            format!(
                                "Delimiter '{}' is not valid. Use one of: ',', '|', or '\\t'",
                                other
                            ),
                            call.head,
                        ),
                    );
                }
            }
        } else {
            Delimiter::Comma
        };

        let space_count = call.get_flag::<i64>("indent-spaces")?.unwrap_or(2) as usize;
        let path_expansion_mode = match call.get_flag::<String>("expand-paths")?.as_deref() {
            Some("off") | None => PathExpansionMode::Off,
            Some("safe") => PathExpansionMode::Safe,
            Some(other) => {
                return Err(LabeledError::new("Invalid Path Expansion Mode".to_string())
                    .with_label(
                        format!(
                            "Path expansion mode '{}' is not valid. Use one of: 'off', 'safe'",
                            other
                        ),
                        call.head,
                    ));
            }
        };
        let strict_mode = call.has_flag("strict").unwrap_or(true);
        let coerce_types = call.has_flag("coerce-types").unwrap_or(true);

        let input_str = input
            .as_str()
            .map_err(|e| {
                LabeledError::new("Input Error".to_string()).with_label(
                    format!("Failed to convert input to string: {}", e),
                    call.head,
                )
            })?
            .replace("\r\n", "\n");

        let toon_decode_options = DecodeOptions::new()
            .with_delimiter(delimiter)
            .with_strict(strict_mode)
            .with_coerce_types(coerce_types)
            .with_indent(Indent::Spaces(space_count))
            .with_expand_paths(path_expansion_mode);

        let decoded_value: serde_json::Value =
            decode(&input_str, &toon_decode_options).map_err(|e| {
                LabeledError::new("Toon Decoding Error".to_string()).with_label(
                    format!(
                        "Failed to decode input '{}' from toon format: {}",
                        input_str, e
                    ),
                    call.head,
                )
            })?;

        let decoded_value_str = serde_json::to_string(&decoded_value).map_err(|e| {
            LabeledError::new("Serialization Error".to_string()).with_label(
                format!("Failed to serialize decoded value to string: {}", e),
                call.head,
            )
        })?;

        let Some(decl_id) = engine.find_decl("from json")? else {
            return Err(LabeledError::new(
                "Could not find 'from json' declaration".to_string(),
            ));
        };

        let from_json = engine.call_decl(
            decl_id,
            EvaluatedCall::new(call.head),
            PipelineData::Value(Value::string(decoded_value_str, call.head), None),
            true,
            false,
        )?;

        let result_value = from_json.into_value(call.head)?;
        Ok(result_value)
    }
}

#[test]
fn test_examples() -> Result<(), nu_protocol::ShellError> {
    use nu_plugin_test_support::PluginTest;

    // This will automatically run the examples specified in your command and compare their actual
    // output against what was specified in the example.
    //
    // We recommend you add this test to any other commands you create, or remove it if the examples
    // can't be tested this way.

    PluginTest::new("toon", ToonPlugin.into())?.test_command_examples(&FromToon)
}
