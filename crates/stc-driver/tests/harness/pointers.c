/* M5 pointer (STC-POINTER) library harness: ADR, SIZEOF, arithmetic, casts. */

typedef int (*int_fn)(void);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int_fn ptr_global = (int_fn)stc_host_symbol("PtrGlobal");
  int_fn ptr_index = (int_fn)stc_host_symbol("PtrIndex");
  int_fn ptr_arith = (int_fn)stc_host_symbol("PtrArith");
  int_fn ptr_param = (int_fn)stc_host_symbol("PtrParam");
  int_fn ptr_result = (int_fn)stc_host_symbol("PtrResult");
  int_fn ptr_conv = (int_fn)stc_host_symbol("PtrConv");
  int_fn ptr_size = (int_fn)stc_host_symbol("PtrSize");
  int_fn ptr_string = (int_fn)stc_host_symbol("PtrString");
  int_fn ptr_array2 = (int_fn)stc_host_symbol("PtrArray2");
  int_fn ptr_field = (int_fn)stc_host_symbol("PtrField");
  int_fn ptr_ptr = (int_fn)stc_host_symbol("PtrPtr");
  int_fn ptr_direct = (int_fn)stc_host_symbol("PtrDirect");
  int_fn ptr_array_of_ptrs = (int_fn)stc_host_symbol("PtrArrayOfPtrs");
  int_fn ptr_null = (int_fn)stc_host_symbol("PtrNull");
  int_fn ptr_leaf = (int_fn)stc_host_symbol("PtrLeaf");
  int_fn ptr_inout = (int_fn)stc_host_symbol("PtrInOut");
  if (!ptr_global) return 40;
  if (!ptr_index) return 41;
  if (!ptr_arith) return 42;
  if (!ptr_param) return 43;
  if (!ptr_result) return 44;
  if (!ptr_conv) return 45;
  if (!ptr_size) return 46;
  if (!ptr_string) return 47;
  if (!ptr_array2) return 48;
  if (!ptr_field) return 49;
  if (!ptr_ptr) return 50;
  if (!ptr_direct) return 51;
  if (!ptr_array_of_ptrs) return 52;
  if (!ptr_null) return 53;
  if (!ptr_leaf) return 54;
  if (!ptr_inout) return 55;

  if (ptr_global() != 14) return 1;
  if (ptr_index() != 14) return 2;
  if (ptr_arith() != 29) return 3;
  if (ptr_param() != 15) return 4;
  if (ptr_result() != 6) return 5;
  if (ptr_conv() != 12) return 6;
  if (ptr_size() != 38) return 7;
  if (ptr_string() != 14) return 8;
  if (ptr_array2() != 7) return 9;
  if (ptr_field() != 65) return 10;
  if (ptr_ptr() != 8) return 11;
  if (ptr_direct() != 42) return 12;
  if (ptr_array_of_ptrs() != 9) return 13;
  if (ptr_null() != 1101) return 14;
  if (ptr_leaf() != 7) return 15;
  if (ptr_inout() != 22) return 16;

  if (stc_host_cycle() != 0) return 20;
  if (stc_host_shutdown() != 0) return 21;
  return 0;
}
