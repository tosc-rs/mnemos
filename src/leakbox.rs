use core::ptr::NonNull;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::UnsafeCell,
    mem::MaybeUninit,
};

use crate::{
    dictionary::{BuiltinEntry}, input::WordStrBuf, output::OutputBuf, word::Word, CallContext, Forth,
};

#[cfg(feature = "async")]
use crate::{AsyncForth, dictionary::{AsyncBuiltins}};

// Helper type that will un-leak the buffer once it is dropped.
pub struct LeakBox<T> {
    ptr: *mut UnsafeCell<MaybeUninit<T>>,
    len: usize,
}

impl<T> LeakBox<T> {
    pub fn new(len: usize) -> Self {
        Self {
            ptr: unsafe {
                System
                    .alloc(Layout::array::<UnsafeCell<MaybeUninit<T>>>(len).unwrap())
                    .cast()
            },
            len,
        }
    }

    pub fn ptr(&self) -> *mut T {
        self.ptr.cast()
    }

    pub fn as_non_null(&self) -> NonNull<T> {
        #[cfg(debug_assertions)]
        let res = NonNull::new(self.ptr.cast::<T>()).unwrap();
        #[cfg(not(debug_assertions))]
        let res = unsafe { NonNull::new_unchecked(self.ptr.cast::<T>()) };

        res
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl<T> Drop for LeakBox<T> {
    fn drop(&mut self) {
        unsafe {
            System.dealloc(
                self.ptr.cast(),
                Layout::array::<UnsafeCell<MaybeUninit<T>>>(self.len).unwrap(),
            )
        }
    }
}

#[derive(Debug)]
pub struct LBForthParams {
    pub data_stack_elems: usize,
    pub return_stack_elems: usize,
    pub control_stack_elems: usize,
    pub input_buf_elems: usize,
    pub output_buf_elems: usize,
    pub dict_buf_elems: usize,
}

impl Default for LBForthParams {
    fn default() -> Self {
        Self {
            data_stack_elems: 256,
            return_stack_elems: 256,
            control_stack_elems: 256,
            input_buf_elems: 256,
            output_buf_elems: 256,
            dict_buf_elems: 4096,
        }
    }
}

pub struct LBForth<T: 'static> {
    pub forth: Forth<T>,
    _payload_dstack: LeakBox<Word>,
    _payload_rstack: LeakBox<Word>,
    _payload_cstack: LeakBox<CallContext<T>>,
    _input_buf: LeakBox<u8>,
    _output_buf: LeakBox<u8>,
    _dict_buf: LeakBox<u8>,
}

#[cfg(feature = "async")]
pub struct AsyncLBForth<T: 'static, D> {
    pub forth: AsyncForth<T, D>,
    _payload_dstack: LeakBox<Word>,
    _payload_rstack: LeakBox<Word>,
    _payload_cstack: LeakBox<CallContext<T>>,
    _input_buf: LeakBox<u8>,
    _output_buf: LeakBox<u8>,
    _dict_buf: LeakBox<u8>,
}

impl<T: 'static> LBForth<T> {
    pub fn from_params(
        params: LBForthParams,
        host_ctxt: T,
        builtins: &'static [BuiltinEntry<T>],
    ) -> Self {
        let _payload_dstack: LeakBox<Word> = LeakBox::new(params.data_stack_elems);
        let _payload_rstack: LeakBox<Word> = LeakBox::new(params.return_stack_elems);
        let _payload_cstack: LeakBox<CallContext<T>> = LeakBox::new(params.control_stack_elems);
        let _input_buf: LeakBox<u8> = LeakBox::new(params.input_buf_elems);
        let _output_buf: LeakBox<u8> = LeakBox::new(params.output_buf_elems);
        let _dict_buf: LeakBox<u8> = LeakBox::new(params.dict_buf_elems);

        let input = WordStrBuf::new(_input_buf.ptr(), _input_buf.len());
        let output = OutputBuf::new(_output_buf.ptr(), _output_buf.len());
        let forth = unsafe {
            Forth::<T>::new(
                (_payload_dstack.ptr(), _payload_dstack.len()),
                (_payload_rstack.ptr(), _payload_rstack.len()),
                (_payload_cstack.ptr(), _payload_cstack.len()),
                (_dict_buf.ptr(), _dict_buf.len()),
                input,
                output,
                host_ctxt,
                builtins,
            )
            .unwrap()
        };

        Self {
            forth,
            _payload_dstack,
            _payload_rstack,
            _payload_cstack,
            _input_buf,
            _output_buf,
            _dict_buf,
        }
    }
}

#[cfg(feature = "async")]
impl<T, D> AsyncLBForth<T, D>
where
    T: 'static,
    D: for<'forth> AsyncBuiltins<'forth, T>,
{
    pub fn from_params(
        params: LBForthParams,
        host_ctxt: T,
        sync_builtins: &'static [BuiltinEntry<T>],
        dispatcher: D
    ) -> Self {
        let _payload_dstack: LeakBox<Word> = LeakBox::new(params.data_stack_elems);
        let _payload_rstack: LeakBox<Word> = LeakBox::new(params.return_stack_elems);
        let _payload_cstack: LeakBox<CallContext<T>> = LeakBox::new(params.control_stack_elems);
        let _input_buf: LeakBox<u8> = LeakBox::new(params.input_buf_elems);
        let _output_buf: LeakBox<u8> = LeakBox::new(params.output_buf_elems);
        let _dict_buf: LeakBox<u8> = LeakBox::new(params.dict_buf_elems);

        let input = WordStrBuf::new(_input_buf.ptr(), _input_buf.len());
        let output = OutputBuf::new(_output_buf.ptr(), _output_buf.len());
        let forth = unsafe {
            AsyncForth::<T, D>::new(
                (_payload_dstack.ptr(), _payload_dstack.len()),
                (_payload_rstack.ptr(), _payload_rstack.len()),
                (_payload_cstack.ptr(), _payload_cstack.len()),
                (_dict_buf.ptr(), _dict_buf.len()),
                input,
                output,
                host_ctxt,
                sync_builtins,
                dispatcher,
            )
            .unwrap()
        };

        Self {
            forth,
            _payload_dstack,
            _payload_rstack,
            _payload_cstack,
            _input_buf,
            _output_buf,
            _dict_buf,
        }
    }
}
