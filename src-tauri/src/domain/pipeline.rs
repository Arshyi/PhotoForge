use super::EditOperation;
use crate::error::AppError;

#[derive(Debug, Default)]
pub struct EditPipeline {
    operations: Vec<EditOperation>,
    undo_stack: Vec<Vec<EditOperation>>,
    redo_stack: Vec<Vec<EditOperation>>,
}

impl EditPipeline {
    pub fn operations(&self) -> &[EditOperation] {
        &self.operations
    }

    pub fn replace(&mut self, operations: Vec<EditOperation>) -> Result<(), AppError> {
        for operation in &operations {
            operation.validate()?;
        }
        if operations != self.operations {
            self.undo_stack.push(self.operations.clone());
            self.operations = operations;
            self.redo_stack.clear();
        }
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.operations.clone());
        self.operations = previous;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.operations.clone());
        self.operations = next;
        true
    }

    pub fn reset(&mut self) -> bool {
        if self.operations.is_empty() {
            return false;
        }
        self.undo_stack.push(self.operations.clone());
        self.operations.clear();
        self.redo_stack.clear();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_operation_order() {
        let mut pipeline = EditPipeline::default();
        pipeline
            .replace(vec![
                EditOperation::Brightness { amount: 0.2 },
                EditOperation::Grayscale,
            ])
            .unwrap();
        assert!(matches!(
            pipeline.operations()[0],
            EditOperation::Brightness { .. }
        ));
        assert!(matches!(pipeline.operations()[1], EditOperation::Grayscale));
    }

    #[test]
    fn supports_undo_redo_and_reset() {
        let mut pipeline = EditPipeline::default();
        pipeline.replace(vec![EditOperation::Sepia]).unwrap();
        assert!(pipeline.undo());
        assert!(pipeline.operations().is_empty());
        assert!(pipeline.redo());
        assert_eq!(pipeline.operations(), &[EditOperation::Sepia]);
        assert!(pipeline.reset());
        assert!(pipeline.operations().is_empty());
    }

    #[test]
    fn rejects_invalid_parameters() {
        let mut pipeline = EditPipeline::default();
        assert!(pipeline
            .replace(vec![EditOperation::Gamma { value: 0.0 }])
            .is_err());
    }
}
