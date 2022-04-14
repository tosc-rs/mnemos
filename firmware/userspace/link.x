/* Anachro Userspace Linker Script                                              */
/*                                                                              */
/* Lovingly borrowed from cortex-m-rt.                                          */
/* https://github.com/rust-embedded/cortex-m/blob/master/cortex-m-rt/link.x.in  */
/*                                                                              */
/* cortex-m-rt licensed under the following MIT license:                        */
/*                                                                              */
/* Copyright (c) 2016 Jorge Aparicio                                            */
/*                                                                              */
/* Permission is hereby granted, free of charge, to any                         */
/* person obtaining a copy of this software and associated                      */
/* documentation files (the "Software"), to deal in the                         */
/* Software without restriction, including without                              */
/* limitation the rights to use, copy, modify, merge,                           */
/* publish, distribute, sublicense, and/or sell copies of                       */
/* the Software, and to permit persons to whom the Software                     */
/* is furnished to do so, subject to the following                              */
/* conditions:                                                                  */
/*                                                                              */
/* The above copyright notice and this permission notice                        */
/* shall be included in all copies or substantial portions                      */
/* of the Software.                                                             */
/*                                                                              */
/* THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF                        */
/* ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED                      */
/* TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A                          */
/* PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT                          */
/* SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY                     */
/* CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION                      */
/* OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR                      */
/* IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER                          */
/* DEALINGS IN THE SOFTWARE.                                                    */

/* # Developer notes

- Symbols that start with a double underscore (__) are considered "private"

- Symbols that start with a single underscore (_) are considered "semi-public"; they can be
  overridden in a user linker script, but should not be referred from user code (e.g. `extern "C" {
  static mut __sbss }`).

- `EXTERN` forces the linker to keep a symbol in the final binary. We use this to make sure a
  symbol if not dropped if it appears in or near the front of the linker arguments and "it's not
  needed" by any of the preceding objects (linker arguments)

- `PROVIDE` is used to provide default values that can be overridden by a user linker script

- On alignment: it's important for correctness that the VMA boundaries of both .bss and .data *and*
  the LMA of .data are all 4-byte aligned. These alignments are assumed by the RAM initialization
  routine. There's also a second benefit: 4-byte aligned boundaries means that you won't see
  "Address (..) is out of bounds" in the disassembly produced by `objdump`.
*/

/* Provides information about the memory layout of the device       */
/*                                                                  */
/* TODO: Should we just combine this space, and let the apps        */
/* their own "memory.x", which declares the 'flash' and RAM space   */
/* they need? This would allow us to support smaller apps (faster   */
/* to load, and portable to systems with variable amounts of memory */
/* available for userspace.                                         */
/*
/* This information could be encoded in the program header, sort of */
/* like how the vector table stores stack and program start         */
/* location information for the hardware. Hardcode for now.         */
/*                                                                  */
/* /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\  */
/* NOTE: _MUST_ be kept in sync with the kernel linker script! Make */
/* sure you update both at the same time!
/* /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\  */
/*                                                                  */
MEMORY
{
  /* This is the 'app program' space, which will be loaded and run  */
  /* in "userspace".                                                */
  /*                                                                */
  /* NOTE: This comes FIRST, because in Cortex-M, RAM always(?)     */
  /* starts here, which means this should be more flexible/portable */
  /* Since the kernel is target-specific anyway (for now?), it      */
  /* doesn't really care where it lives.                            */
  APP   : ORIGIN = 0x20000000, LENGTH = 128K
}

INCLUDE stack.x

/* # Entry point = reset vector */
EXTERN(__ENTRY_POINT);
EXTERN(SYSCALL_IN_PTR);
EXTERN(SYSCALL_IN_LEN);
EXTERN(SYSCALL_OUT_PTR);
EXTERN(SYSCALL_OUT_LEN);

/* # Sections */
SECTIONS
{
  /* Default to a 16K stack size */
  PROVIDE(_stack_size = 0x4000);
  PROVIDE(_stack_start = __eapp + _stack_size);

  /* ## Sections in APP */
  .bridge : ALIGN(4)
  {
    __sbridge = .;
    /* Initial Stack Pointer (SP) value */
    *(.bridge.syscall_in.ptr .bridge.syscall_in.ptr.*);
    *(.bridge.syscall_in.len .bridge.syscall_in.len.*);
    *(.bridge.syscall_out.ptr .bridge.syscall_out.ptr.*);
    *(.bridge.syscall_out.len .bridge.syscall_out.len.*);

    KEEP(*(.bridge.syscall_in.ptr .bridge.syscall_in.ptr.*));
    KEEP(*(.bridge.syscall_in.len .bridge.syscall_in.len.*));
    KEEP(*(.bridge.syscall_out.ptr .bridge.syscall_out.ptr.*));
    KEEP(*(.bridge.syscall_out.len .bridge.syscall_out.len.*));

    __start_app_ram = .;
  } > APP

  /* ### Vector table */
  .anachro_table __start_app_ram :
  {
    __satable = .;
    /* Headers for the header gods! */

    LONG(__etext);        /* End of text section. 0x2000_0000..__etext will be copied             */
    LONG(__srodata);      /* Start of .rodata section. __srodata..(__srodata + (__edata-__sdata)) */
                          /* will be copied to __sdata..__edata                                   */
    LONG(__sdata);        /* Start of .data section. __srodata will be copied starting here       */
    LONG(__edata);        /* End of .data section. __srodata will be copied ending here           */
    LONG(__sbss);         /* Start of .bss section. The runtime will zero starting here           */
    LONG(__ebss);         /* End of .bss section. The runtime will zero up to here                */
    LONG(_stack_start);   /* Stack start location. The PSP will be placed here                    */

    /* Reset vector */
    KEEP(*(.anachro_table.entry_point)); /* this is the `__ENTRY_POINT` symbol */
    __ENTRY_POINT = .;
  } > APP

  /* ### .text */
  .text :
  {
    . = ALIGN(4);
    __stext = .;
    *(.text .text.*);
    . = ALIGN(4); /* Pad .text to the alignment to workaround overlapping load section bug in old lld */
    __etext = .;
  } > APP

  /* ### .rodata */
  .rodata : ALIGN(4)
  {
    . = ALIGN(4);
    __srodata = .;
    *(.rodata .rodata.*);

    /* 4-byte align the end (VMA) of this section.
       This is required by LLD to ensure the LMA of the following .data
       section will have the correct alignment. */
    . = ALIGN(4);
    __erodata = .;
  } > APP

  /* ## Sections in ARAM */
  /* ### .data */
  .data : ALIGN(4)
  {
    . = ALIGN(4);
    __sdata = .;
    *(.data .data.*);
    . = ALIGN(4); /* 4-byte align the end (VMA) of this section */
  } > APP

  /* Allow sections from user `memory.x` injected using `INSERT AFTER .data` to
   * use the .data loading mechanism by pushing __edata. Note: do not change
   * output region or load region in those user sections! */

  . = ALIGN(4);
  __edata = .;

  /* ### .bss */
  .bss (NOLOAD) : ALIGN(4)
  {
    . = ALIGN(4);
    __sbss = .;
    *(.bss .bss.*);
    *(COMMON); /* Uninitialized C statics */
    . = ALIGN(4); /* 4-byte align the end (VMA) of this section */
  } > APP

  /* Allow sections from user `memory.x` injected using `INSERT AFTER .bss` to
   * use the .bss zeroing mechanism by pushing __ebss. Note: do not change
   * output region or load region in those user sections! */
  . = ALIGN(4);
  __ebss = .;

  /* ### .uninit */
  .uninit (NOLOAD) : ALIGN(4)
  {
    . = ALIGN(4);
    __suninit = .;
    *(.uninit .uninit.*);
    . = ALIGN(4);
    __euninit = .;
  } > APP


  /* ------------------------------------------------ */
  /* End of contents!                                 */
  . = ALIGN(4);
  __eapp = .;

  /* ## .got */
  /* Dynamic relocations are unsupported. This section is only used to detect relocatable code in
     the input files and raise an error if relocatable code is found */
  .got (NOLOAD) :
  {
    KEEP(*(.got .got.*));
  }

  /* ## Discarded sections */
  /DISCARD/ :
  {
    /* Unused exception related info that only wastes space */
    *(.ARM.exidx);
    *(.ARM.exidx.*);
    *(.ARM.extab.*);
  }
}

/* Do not exceed this mark in the error messages below                                    | */
/* # Alignment checks */
ASSERT(ORIGIN(APP) == 0x20000000, "
ERROR(anachro-lnk): the start of the APP region must 0x20000000");

ASSERT(__sdata % 4 == 0 && __edata % 4 == 0, "
BUG(anachro-lnk): .data is not 4-byte aligned");

ASSERT(__sbss % 4 == 0 && __ebss % 4 == 0, "
BUG(anachro-lnk): .bss is not 4-byte aligned");

/* # Position checks */

/* ## .text */
ASSERT(__stext + SIZEOF(.text) < ORIGIN(APP) + LENGTH(APP), "
ERROR(anachro-lnk): The .text section must be placed inside the APP memory.
Set _stext to an address smaller than 'ORIGIN(APP) + LENGTH(APP)'");

/* # Other checks */

ASSERT(__sbridge == ORIGIN(APP), "WHAT");
ASSERT(__satable == ORIGIN(APP) + 16, "NO BRIDGE");
ASSERT(__stext == ORIGIN(APP) + 16 + 32, "__stext wrong!");
ASSERT(_stack_start <= (ORIGIN(APP) + LENGTH(APP)), "
ERROR(anachro-lnk): Application + Stack too big! Consider reducing stack size.");

ASSERT(SIZEOF(.got) == 0, "
ERROR(anachro-lnk): .got section detected in the input object files
Dynamic relocations are not supported. If you are linking to C code compiled using
the 'cc' crate then modify your build script to compile the C code _without_
the -fPIC flag. See the documentation of the `cc::Build.pic` method for details.");
/* Do not exceed this mark in the error messages above                                    | */
