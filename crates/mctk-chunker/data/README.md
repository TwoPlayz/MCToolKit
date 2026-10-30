# Data

- `font_widths.json`: Chunker's `default_font_width_codepoints.json` (MIT, Hive Games), pixel widths of characters in Minecraft's default font.
- `entity_types.tsv`: `type<TAB>Java id<TAB>Bedrock id` per Chunker entity type, produced by evaluating the version conditions of Chunker's `JavaEntityTypeResolver` and `BedrockEntityTypeResolver` for Java 26.3 and Bedrock 1.26.60 (Chunker commit 50aaf6d). A type missing in one edition has an empty column.

