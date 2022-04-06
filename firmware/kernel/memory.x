/* Linker script for the nRF52 - WITHOUT SOFT DEVICE */

/*                                                                  */
/* /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\  */
/* NOTE: _MUST_ be kept in sync with the user linker script! Make   */
/* sure you update both at the same time!
/* /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\ /!\  */
/*                                                                  */
MEMORY
{
  /* NOTE K = KiBi = 1024 bytes                                     */
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K

  /* This is the 'app program' space, which will be loaded and run  */
  /* in "userspace". It also contains the APP RAM and stack.        */
  /*                                                                */
  /* NOTE: This comes FIRST, because in Cortex-M, RAM always(?)     */
  /* starts here, which means this should be more flexible/portable */
  /* Since the kernel is target-specific anyway (for now?), it      */
  /* doesn't really care where it lives.                            */
  APP   : ORIGIN = 0x20000000, LENGTH = 128K

  /* This is the "OS RAM", where the MSP stack will be located.     */
  RAM   : ORIGIN = 0x20020000, LENGTH = 64K

  /* This is the shared HEAP region used by AHEAP                   */
  HEAP  : ORIGIN = 0x20030000, LENGTH = 64K
}

SECTIONS
{
  .aheap (NOLOAD) : ALIGN(4)
  {
    *(.aheap .aheap.*);
    . = ALIGN(4);
  } > HEAP

  .bridge (NOLOAD) : ALIGN(4)
  {
    /* Initial Stack Pointer (SP) value */
    *(.bridge.syscall_in.ptr .bridge.syscall_in.ptr.*);
    *(.bridge.syscall_in.len .bridge.syscall_in.len.*);
    *(.bridge.syscall_out.ptr .bridge.syscall_out.ptr.*);
    *(.bridge.syscall_out.len .bridge.syscall_out.len.*);
    __start_app_ram = .;
  } > APP
}

/* This is where the call stack will be allocated. */
/* The stack is of the full descending type. */
/* You may want to use this variable to locate the call stack and static
   variables in different memory regions. Below is shown the default value */
_app_stack_start = ORIGIN(APP) + LENGTH(APP);

/* You can use this symbol to customize the location of the .text section */
/* If omitted the .text section will be placed right after the .vector_table
   section */
/* This is required only on microcontrollers that store some configuration right
   after the vector table */
/* _stext = ORIGIN(FLASH) + 0x400; */

/* Size of the heap (in bytes) */
/* _heap_size = 1024; */
