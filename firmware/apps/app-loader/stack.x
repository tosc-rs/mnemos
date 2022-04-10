/* You must have a stack.x file, even if you    */
/* accept the defaults.                         */

/* How large is the stack? Defaults to 16KiB    */
/*                                              */
/* _stack_size = 0x4000;                        */
_stack_size = 0x10000;

/* Where should the stack start? Defaults to    */
/* _stack_size bytes after the end of all other */
/* application contents (__eapp), which is four */
/* byte aligned.                                */
/*                                              */
/* _stack_start = __eapp + _stack_size;         */
