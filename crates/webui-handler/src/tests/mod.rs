mod render_attributes;
mod render_core;
mod render_nested;

use std::cell::RefCell;
use webui_protocol::{web_ui_fragment, ConditionExpr, FragmentList, WebUIFragmentAttribute};
use webui_test_utils::test_json;

use crate::*;

// A simple test writer implementation
struct TestWriter {
    content: RefCell<String>,
    ended: RefCell<bool>,
}

impl TestWriter {
    fn new() -> Self {
        Self {
            content: RefCell::new(String::new()),
            ended: RefCell::new(false),
        }
    }

    fn get_content(&self) -> String {
        self.content.borrow().clone()
    }

    fn is_ended(&self) -> bool {
        *self.ended.borrow()
    }
}

impl ResponseWriter for TestWriter {
    fn write(&mut self, content: &str) -> Result<()> {
        self.content.borrow_mut().push_str(content);
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        *self.ended.borrow_mut() = true;
        Ok(())
    }
}
