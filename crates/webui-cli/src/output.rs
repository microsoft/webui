use console::Style;

pub struct Printer {
    pub cyan: Style,
    pub green: Style,
    pub red: Style,
    pub dim: Style,
    pub bold: Style,
}

impl Printer {
    pub fn new() -> Self {
        Self {
            cyan: Style::new().cyan().bold(),
            green: Style::new().green(),
            red: Style::new().red().bold(),
            dim: Style::new().dim(),
            bold: Style::new().bold(),
        }
    }

    pub fn header(&self, title: &str) {
        eprintln!(
            "\n  {} {}\n",
            self.cyan.apply_to("⚡"),
            self.cyan.apply_to(title)
        );
    }

    pub fn field(&self, label: &str, value: &dyn std::fmt::Display) {
        eprintln!(
            "  {} {}",
            self.dim.apply_to(format!("▸ {label:<10}")),
            self.bold.apply_to(value)
        );
    }

    pub fn success(&self, message: &str) {
        eprintln!("  {} {message}", self.green.apply_to("✔"));
    }

    pub fn finish(&self, message: &str) {
        eprintln!("\n  {} {message}\n", self.green.apply_to("✨"));
    }

    pub fn error(&self, err: &anyhow::Error) {
        eprintln!("\n  {} {}", self.red.apply_to("✘"), self.red.apply_to(err));
        for cause in err.chain().skip(1) {
            eprintln!("  {} {cause}", self.dim.apply_to("caused by:"));
        }
    }

    pub fn hint(&self, message: &str) {
        eprintln!("\n  {} {message}", self.dim.apply_to("hint:"));
    }
}

impl Default for Printer {
    fn default() -> Self {
        Self::new()
    }
}
