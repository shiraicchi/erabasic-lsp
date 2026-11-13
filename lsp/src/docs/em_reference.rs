//! Parse EM-Reference markdown files for hover documentation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed documentation from EM-Reference.
#[derive(Debug, Clone)]
pub struct EmReferenceDoc {
    pub name: String,
    pub summary: String,
    pub syntax: Option<String>,
    pub parameters: Vec<ParameterDoc>,
    pub return_type: Option<String>,
    pub return_description: Option<String>,
    pub description: String,
    pub examples: Vec<String>,
    pub see_also: Vec<String>,
}

/// A parameter documentation.
#[derive(Debug, Clone)]
pub struct ParameterDoc {
    pub name: String,
    pub type_: Option<String>,
    pub description: String,
    pub optional: bool,
}

/// EM-Reference documentation index.
#[derive(Debug)]
pub struct EmReferenceIndex {
    docs: HashMap<String, EmReferenceDoc>,
    path: PathBuf,
}

impl EmReferenceIndex {
    /// Create a new index from the EM-Reference directory.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let mut index = Self {
            docs: HashMap::new(),
            path: path.to_path_buf(),
        };
        index.load()?;
        Ok(index)
    }

    /// Load all markdown files from the directory.
    fn load(&mut self) -> std::io::Result<()> {
        let entries = std::fs::read_dir(&self.path)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(doc) = self.parse_markdown(&content) {
                        self.docs.insert(doc.name.to_uppercase(), doc);
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse a markdown file into documentation.
    fn parse_markdown(&self, content: &str) -> Option<EmReferenceDoc> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return None;
        }

        // First line is usually the title
        let title = lines[0].trim_start_matches('#').trim();
        let name = title.split_whitespace().next()?.to_string();

        let mut doc = EmReferenceDoc {
            name: name.clone(),
            summary: String::new(),
            syntax: None,
            parameters: Vec::new(),
            return_type: None,
            return_description: None,
            description: String::new(),
            examples: Vec::new(),
            see_also: Vec::new(),
        };

        let mut current_section = String::new();
        let mut in_code_block = false;
        let mut code_block_content = String::new();

        for line in lines.iter().skip(1) {
            let trimmed = line.trim();

            // Code blocks
            if trimmed.starts_with("```") {
                if in_code_block {
                    // End of code block
                    if current_section == "syntax" {
                        doc.syntax = Some(code_block_content.trim().to_string());
                    } else if current_section == "example" {
                        doc.examples.push(code_block_content.trim().to_string());
                    }
                    code_block_content.clear();
                    in_code_block = false;
                } else {
                    in_code_block = true;
                    // Check what section this code block belongs to
                }
                continue;
            }

            if in_code_block {
                code_block_content.push_str(line);
                code_block_content.push('\n');
                continue;
            }

            // Section headers
            if trimmed.starts_with("## ") {
                current_section = trimmed.trim_start_matches("## ").trim().to_lowercase();
                continue;
            }

            // Skip empty lines
            if trimmed.is_empty() {
                continue;
            }

            // Parse content based on section
            match current_section.as_str() {
                "syntax" | "syntaxes" => {
                    if doc.syntax.is_none() {
                        doc.syntax = Some(trimmed.to_string());
                    }
                }
                "parameters" | "parameter" | "args" | "arguments" => {
                    if let Some(param) = self.parse_parameter_line(trimmed) {
                        doc.parameters.push(param);
                    }
                }
                "returns" | "return" | "return value" => {
                    if doc.return_type.is_none() {
                        // Try to parse type from first line
                        if let Some(paren_end) = trimmed.find(')') {
                            doc.return_type = Some(trimmed[..=paren_end].to_string());
                            doc.return_description =
                                Some(trimmed[paren_end + 1..].trim().to_string());
                        } else {
                            doc.return_description = Some(trimmed.to_string());
                        }
                    }
                }
                "example" | "examples" => {
                    doc.examples.push(trimmed.to_string());
                }
                "see also" | "related" => {
                    doc.see_also.push(trimmed.to_string());
                }
                "description" | "overview" | "" => {
                    if doc.summary.is_empty() {
                        doc.summary = trimmed.to_string();
                    } else {
                        doc.description.push_str(trimmed);
                        doc.description.push('\n');
                    }
                }
                _ => {
                    if doc.description.is_empty() && doc.summary.is_empty() {
                        doc.summary = trimmed.to_string();
                    } else {
                        doc.description.push_str(trimmed);
                        doc.description.push('\n');
                    }
                }
            }
        }

        Some(doc)
    }

    /// Parse a parameter line from markdown.
    fn parse_parameter_line(&self, line: &str) -> Option<ParameterDoc> {
        // Formats: `- name (type): description` or `- name: description`
        let line = line.trim_start_matches("- ").trim_start_matches("* ");

        let (name_type, desc) = line.split_once(':')?;
        let name_type = name_type.trim();
        let desc = desc.trim();

        let (name, type_) = if let Some(paren_start) = name_type.find('(') {
            if let Some(paren_end) = name_type.find(')') {
                let name = name_type[..paren_start].trim().to_string();
                let type_ = name_type[paren_start + 1..paren_end].trim().to_string();
                let optional = type_.ends_with('?');
                let type_ = type_.trim_end_matches('?').trim().to_string();
                (name, if type_.is_empty() { None } else { Some(type_) })
            } else {
                (name_type.to_string(), None)
            }
        } else {
            (name_type.to_string(), None)
        };

        Some(ParameterDoc {
            name,
            type_,
            description: desc.to_string(),
            optional: false,
        })
    }

    /// Get documentation for a function/command.
    pub fn get(&self, name: &str) -> Option<&EmReferenceDoc> {
        self.docs.get(&name.to_uppercase())
    }

    /// Get all documented names.
    pub fn all_names(&self) -> impl Iterator<Item = &String> {
        self.docs.keys()
    }

    /// Format documentation for hover.
    pub fn format_hover(&self, name: &str) -> Option<String> {
        let doc = self.get(name)?;

        let mut output = String::new();

        output.push_str(&format!("# {}\n\n", doc.name));

        if !doc.summary.is_empty() {
            output.push_str(&format!("{}\n\n", doc.summary));
        }

        if let Some(syntax) = &doc.syntax {
            output.push_str(&format!("## Syntax\n```erabasic\n{}\n```\n\n", syntax));
        }

        if !doc.parameters.is_empty() {
            output.push_str("## Parameters\n\n");
            for param in &doc.parameters {
                let type_str = param
                    .type_
                    .as_ref()
                    .map(|t| format!(" ({})", t))
                    .unwrap_or_default();
                let opt_str = if param.optional { " *optional*" } else { "" };
                output.push_str(&format!(
                    "- **{}**{}{}: {}\n",
                    param.name, type_str, opt_str, param.description
                ));
            }
            output.push('\n');
        }

        if let Some(ret) = &doc.return_type {
            output.push_str(&format!("## Returns\n`{}`\n\n", ret));
        }
        if let Some(ret_desc) = &doc.return_description {
            output.push_str(&format!("{}\n\n", ret_desc));
        }

        if !doc.description.is_empty() {
            output.push_str(&format!("## Description\n{}\n\n", doc.description));
        }

        if !doc.examples.is_empty() {
            output.push_str("## Examples\n\n");
            for example in &doc.examples {
                output.push_str(&format!("```erabasic\n{}\n```\n\n", example));
            }
        }

        if !doc.see_also.is_empty() {
            output.push_str("## See Also\n\n");
            for see in &doc.see_also {
                output.push_str(&format!("- {}\n", see));
            }
        }

        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_em_reference_parsing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("PRINT.md");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "# PRINT").unwrap();
        writeln!(file, "Output text to the console.").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "## Syntax").unwrap();
        writeln!(file, "```erabasic").unwrap();
        writeln!(file, "PRINT text").unwrap();
        writeln!(file, "```").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "## Parameters").unwrap();
        writeln!(file, "- text (string): Text to output").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "## Example").unwrap();
        writeln!(file, "```erabasic").unwrap();
        writeln!(file, "PRINT \"Hello World\"").unwrap();
        writeln!(file, "```").unwrap();

        let index = EmReferenceIndex::new(dir.path()).unwrap();
        let doc = index.get("PRINT").unwrap();

        assert_eq!(doc.name, "PRINT");
        assert_eq!(doc.summary, "Output text to the console.");
        assert!(doc.syntax.is_some());
        assert_eq!(doc.parameters.len(), 1);
        assert_eq!(doc.parameters[0].name, "text");

        let hover = index.format_hover("PRINT").unwrap();
        assert!(hover.contains("PRINT"));
        assert!(hover.contains("Syntax"));
        assert!(hover.contains("Example"));
    }
}
