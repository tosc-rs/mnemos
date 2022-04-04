/* Linker script for the nRF52 - WITHOUT SOFT DEVICE */
MEMORY
{
  /* NOTE K = KiBi = 1024 bytes                                     */
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K

  /* This is the 'app program' space, which will be loaded and run  */
  /* in "userspace".                                                */
  /*                                                                */
  /* NOTE: This comes FIRST, because in Cortex-M, RAM always(?)     */
  /* starts here, which means this should be more flexible/portable */
  /* Since the kernel is target-specific anyway (for now?), it      */
  /* doesn't really care where it lives.                            */
  APP   : ORIGIN = 0x20000000, LENGTH = 64K

  /* This is the "APP RAM", where the PSP stack will be located.    */
  /*                                                                */
  /* NOTE: This is also SECOND, for the same reason APP is FIRST.   */
  /*                                                                */
  /* NOTE: In the future, maybe we should just congeal the two app  */
  /* regions ('APP' and 'ARAM') so that there is 128K available,    */
  /* and it is up to the app to set (and report?) what the split    */
  /* between the two regions are. For now, just hardcode the two.   */
  ARAM  : ORIGIN = 0x20010000, LENGTH = 64K

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
    *(.bridge.syscall_in .bridge.syscall_in.*);
    *(.bridge.syscall_out .bridge.syscall_out.*);
    __start_app_ram = .;
  } > ARAM
}

/* This is where the call stack will be allocated. */
/* The stack is of the full descending type. */
/* You may want to use this variable to locate the call stack and static
   variables in different memory regions. Below is shown the default value */
/* _stack_start = ORIGIN(RAM) + LENGTH(RAM); */

/* You can use this symbol to customize the location of the .text section */
/* If omitted the .text section will be placed right after the .vector_table
   section */
/* This is required only on microcontrollers that store some configuration right
   after the vector table */
/* _stext = ORIGIN(FLASH) + 0x400; */

/* Size of the heap (in bytes) */
/* _heap_size = 1024; */
