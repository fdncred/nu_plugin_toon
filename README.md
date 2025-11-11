# nu_plugin_toon

This is a [Nushell](https://nushell.sh/) plugin that implements the [toon format](https://github.com/toon-format/toon-rust) and has two commands `from toon` and `to toon`.

## Installing

```nushell
> cargo install --path .
```

## Usage

### to toon
```nushell
to toon --help
Convert nushell input to toon format

'Under the hood' to toon calls the to json command on anything you pipe into it before trying to convert the input into the toon format

Usage:
  > to toon {flags}

Flags:
  -h, --help: Display the help message for this command
  -d, --delimiter <string>: Delimiter to use: ',', '|', or "\t" (default is ',')
  -i, --indent-spaces <int>: The number of spaces to indent (default is 2)
  -k, --key-folding-mode <string>: Keyfolding mode, 'off' or 'safe'. When set to 'Safe', single-key object chains will be folded into dotted-path notation if all safety requirements are met
  -f, --folding-depth <int>: Set maximum depth for key folding

Examples:
  Convert ls output to toon format
  > ls | to toon

  Convert table literal to toon format
  > [[col1 col2 col3]; [moe larry curly] [larry curly moe]] | to toon
  [2]{col1,col2,col3}:
    moe,larry,curly
    larry,curly,moe
```    

### from toon
```nushell
from toon --help                                                                                                                                                        13  12:41:11 PM
Convert toon formatted text to nushell values

'Under the hood' from toon calls the from json command after decoding the toon formatted input into JSON. Parsing the toon format is set to strict. Make sure the toon output has 2 spaces, is delimited by commas, and uses \n as a line separator and not \r\n even on Windows

Usage:
  > from toon {flags}

Flags:
  -h, --help: Display the help message for this command
  -d, --delimiter <string>: Delimiter to use: ',', '|', or "\t" (default is ',')
  -e, --expand-paths <string>: Enable path expansion 'off' or 'safe' (defaults to 'off'). When set to 'Safe', dotted keys will be expanded into nested objects if all segments are IdentifierSegments
  -s, --strict: Enable or disable strict mode (validates array lengths, indentation, etc) (default: true)
  -i, --indent-spaces <int>: The number of spaces to indent (default is 2)
  -c, --coerce-types: Enable or disable type coercion (strings like “123” -> numbers) (default: true)

Examples:
  Convert ls output to toon format and round trip back to nushell values
  > ls | to toon | from toon

  Convert toon formatted text to nushell values
  > "[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe\n" | from toon
  ╭─#─┬─col1──┬─col2──┬─col3──╮
  │ 0 │ moe   │ larry │ curly │
  │ 1 │ larry │ curly │ moe   │
  ╰───┴───────┴───────┴───────╯
```  