use crate::ToonPlugin;
use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, Example, LabeledError, PipelineData, Signature, Value};
use toon_format::{encode, Delimiter, EncodeOptions, Indent};

pub struct ToToon;

impl SimplePluginCommand for ToToon {
    type Plugin = ToonPlugin;

    fn name(&self) -> &str {
        "to toon"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name()).category(Category::Experimental)
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
        let json_str = json_value.as_str()?;

        // let str_input = input.as_str()?;
        // eprintln!("  Input Data: {json_str_input}\n");

        let json_value = serde_json::from_str::<serde_json::Value>(json_str).map_err(|e| {
            LabeledError::new("JSON Parsing Error".to_string())
                .with_label(format!("Failed to parse input as JSON: {}", e), call.head)
        })?;
        // let data = json!({ "users": [ { "id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]});

        let toon_encode_options = EncodeOptions::new()
            .with_delimiter(Delimiter::Comma)
            .with_indent(Indent::Spaces(2))
            // .with_length_marker('#')
            .with_spaces(2);

        let toon = encode(&json_value, &toon_encode_options).map_err(|e| {
            LabeledError::new("Toon Encoding Error".to_string()).with_label(
                format!("Failed to encode data '{}' to toon format: {}", json_str, e),
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
