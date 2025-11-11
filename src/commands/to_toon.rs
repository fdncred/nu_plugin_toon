use crate::ToonPlugin;
use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, Example, LabeledError, PipelineData, Signature, SyntaxShape, Value};
use toon_format::{encode, types::KeyFoldingMode, Delimiter, EncodeOptions, Indent};

pub struct ToToon;

impl SimplePluginCommand for ToToon {
    type Plugin = ToonPlugin;

    fn name(&self) -> &str {
        "to toon"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                "delimiter",
                SyntaxShape::String,
                "Delimiter to use: ',', '|', or \"\\t\" (default is ',')",
                Some('d'),
            )
            .named(
                "indent-spaces",
                SyntaxShape::Int,
                "The number of spaces to indent (default is 2)",
                Some('i'),
            )
            .named(
                "key-folding-mode",
                SyntaxShape::String,
                "Keyfolding mode, 'off' or 'safe'. When set to 'Safe', single-key object chains will be folded into dotted-path notation if all safety requirements are met",
                Some('k'),
            )
            .named(
                "folding-depth",
                SyntaxShape::Int,
                "Set maximum depth for key folding",
                Some('f'),
            )
            .switch("raw", "Don't call internal `to json` command and just pass json as the input", Some('r'))
            .category(Category::Experimental)
    }

    fn description(&self) -> &str {
        "Convert nushell input to toon format"
    }

    fn extra_description(&self) -> &str {
        "'Under the hood' `to toon` calls the `to json` command on anything you pipe into it before trying to convert the input into the toon format"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Convert `ls` output to toon format",
                example: "ls | to toon",
                result: None,
            },
            Example {
                description: "Convert table literal to toon format",
                example: "[[col1 col2 col3]; [moe larry curly] [larry curly moe]] | to toon",
                result: Some(Value::test_string(
                    "[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe\n",
                )),
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
        let raw = call.has_flag("raw")?;
        let space_count = call.get_flag::<i64>("indent-spaces")?.unwrap_or(2) as usize;
        let folding_mode = if let Some(folding) = call.get_flag::<String>("key-folding-mode")? {
            match folding.to_ascii_lowercase().as_str() {
                "off" => KeyFoldingMode::Off,
                "safe" => KeyFoldingMode::Safe,
                other => {
                    return Err(
                        LabeledError::new("Invalid Folding Mode".to_string()).with_label(
                            format!(
                                "Folding mode '{}' is not valid. Use one of: 'off' or 'safe'",
                                other
                            ),
                            call.head,
                        ),
                    );
                }
            }
        } else {
            KeyFoldingMode::Off
        };

        let folding_depth = call
            .get_flag::<usize>("folding-depth")?
            .unwrap_or(usize::MAX);

        let json_data = if raw {
            input
                .as_str()
                .or_else(|_| {
                    Err(LabeledError::new("Invalid Input".to_string()).with_label(
                        "Expected a string input when using the --raw flag".to_string(),
                        call.head,
                    ))
                })?
                .to_string()
        } else {
            // Get the 'to json' declaration
            let Some(decl_id) = engine.find_decl("to json")? else {
                return Err(LabeledError::new(
                    "Could not find 'to json' declaration".to_string(),
                ));
            };

            // Call 'to json' on the input value
            let to_json = engine.call_decl(
                decl_id,
                EvaluatedCall::new(call.head),
                PipelineData::Value(input.clone(), None),
                true,
                false,
            )?;

            let json_value = to_json.into_value(call.head)?;
            json_value.as_str()?.to_string()
        };

        let json_value = serde_json::from_str::<serde_json::Value>(&json_data).map_err(|e| {
            LabeledError::new("JSON Parsing Error".to_string())
                .with_label(format!("Failed to parse input as JSON: {}", e), call.head)
        })?;

        let toon_encode_options = EncodeOptions::new()
            .with_delimiter(delimiter)
            .with_indent(Indent::Spaces(space_count))
            .with_key_folding(folding_mode)
            .with_flatten_depth(folding_depth);

        let toon = encode(&json_value, &toon_encode_options).map_err(|e| {
            LabeledError::new("Toon Encoding Error".to_string()).with_label(
                format!(
                    "Failed to encode data '{}' to toon format: {}",
                    json_data, e
                ),
                call.head,
            )
        })?;

        Ok(Value::string(toon, call.head))
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

    PluginTest::new("toon", ToonPlugin.into())?.test_command_examples(&ToToon)
}
