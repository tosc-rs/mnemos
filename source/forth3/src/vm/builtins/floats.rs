use crate::{
    vm::{ForthResult, InterpretAction},
    word::Word,
    Error, Forth,
};
use core::{fmt::Write, ops::Neg};

impl<T: 'static> Forth<T> {
    pub fn float_div_mod(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        if unsafe { a.float == 0.0 } {
            return Err(Error::DivideByZero);
        }
        let rem = unsafe { Word::float(b.float % a.float) };
        self.data_stack.push(rem)?;
        let val = unsafe { Word::float(b.float / a.float) };
        self.data_stack.push(val)?;
        Ok(InterpretAction::Done)
    }

    pub fn float_div(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe {
            if a.float == 0.0 {
                return Err(Error::DivideByZero);
            }
            Word::float(b.float / a.float)
        };
        self.data_stack.push(val)?;
        Ok(InterpretAction::Done)
    }

    pub fn float_modu(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe {
            if a.float == 0.0 {
                return Err(Error::DivideByZero);
            }
            Word::float(b.float % a.float)
        };
        self.data_stack.push(val)?;
        Ok(InterpretAction::Done)
    }

    pub fn float_pop_print(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        write!(&mut self.output, "{} ", unsafe { a.float })?;
        Ok(InterpretAction::Done)
    }

    pub fn float_add(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float + b.float }))?;
        Ok(InterpretAction::Done)
    }

    pub fn float_mul(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float * b.float }))?;
        Ok(InterpretAction::Done)
    }

    #[cfg(feature = "use-std")]
    pub fn float_abs(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float.abs() }))?;
        Ok(InterpretAction::Done)
    }

    #[cfg(not(feature = "use-std"))]
    pub fn float_abs(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        self.data_stack.push(Word::float(unsafe {
            if a.float.is_sign_negative() {
                a.float.neg()
            } else {
                a.float
            }
        }))?;
        Ok(InterpretAction::Done)
    }

    pub fn float_negate(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float.neg() }))?;
        Ok(InterpretAction::Done)
    }

    pub fn float_min(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float.min(b.float) }))?;
        Ok(InterpretAction::Done)
    }

    pub fn float_max(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { a.float.max(b.float) }))?;
        Ok(InterpretAction::Done)
    }

    pub fn float_minus(&mut self) -> ForthResult {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::float(unsafe { b.float - a.float }))?;
        Ok(InterpretAction::Done)
    }
}
