use std::io::{stdin, stdout, Write};

use forth3::{
    leakbox::{LBForth, LBForthParams},
    Forth,
};

fn main() {
    let params = LBForthParams {
        data_stack_elems: 1024,
        return_stack_elems: 1024,
        control_stack_elems: 64,
        input_buf_elems: 1024,
        output_buf_elems: 4096,
        dict_buf_elems: 16 * 1024,
    };
    let mut lbf = LBForth::from_params(params, (), Forth::FULL_BUILTINS);
    let forth = &mut lbf.forth;

    let mut inp = String::new();
    loop {
        print!("> ");
        stdout().flush().unwrap();
        stdin().read_line(&mut inp).unwrap();
        forth.input.fill(&inp).unwrap();
        match forth.process_line() {
            Ok(_) => {
                print!("{}", forth.output.as_str());
            }
            Err(e) => {
                println!();
                println!("Input failed. Error: {:?}", e);
                println!("Unprocessed tokens:");
                while let Some(tok) = forth.input.cur_word() {
                    print!("'{}', ", tok);
                    forth.input.advance();
                }
                println!();
            }
        }

        inp.clear();
        forth.output.clear();
    }
}
