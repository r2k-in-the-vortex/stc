/* M5 aggregate library harness: struct/array/string globals through exports. */

typedef int (*int_fn)(void);
typedef int (*int2_fn)(int, int);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int2_fn set_point = (int2_fn)stc_host_symbol("SetPoint");
  int_fn get_sum = (int_fn)stc_host_symbol("GetSum");
  int2_fn set_elem = (int2_fn)stc_host_symbol("SetElem");
  int_fn set_text = (int_fn)stc_host_symbol("SetText");
  int_fn copy_test = (int_fn)stc_host_symbol("CopyTest");
  int_fn init_sum = (int_fn)stc_host_symbol("InitSum");
  int_fn multi = (int_fn)stc_host_symbol("Multi");
  int_fn run = (int_fn)stc_host_symbol("Run");
  int_fn bump_init = (int_fn)stc_host_symbol("BumpInit");
  int_fn sum_bp = (int_fn)stc_host_symbol("SumBp");
  int_fn ref_test = (int_fn)stc_host_symbol("RefTest");
  int_fn run_conf = (int_fn)stc_host_symbol("RunConf");
  int_fn tick = (int_fn)stc_host_symbol("Tick");
  int_fn run_oop = (int_fn)stc_host_symbol("RunOop");
  int_fn math_test = (int_fn)stc_host_symbol("MathTest");
  int_fn string_ops = (int_fn)stc_host_symbol("StringOps");
  int_fn wide_ops = (int_fn)stc_host_symbol("WideOps");
  int_fn unicode_ops = (int_fn)stc_host_symbol("UnicodeOps");
  int_fn run_conf2 = (int_fn)stc_host_symbol("RunConf2");
  if (!set_point) return 40;
  if (!get_sum) return 41;
  if (!set_elem) return 42;
  if (!set_text) return 43;
  if (!copy_test) return 44;
  if (!init_sum) return 45;
  if (!multi) return 46;
  if (!run) return 47;
  if (!bump_init) return 48;
  if (!sum_bp) return 49;
  if (!ref_test) return 50;
  if (!run_conf) return 51;
  if (!tick) return 52;
  if (!run_oop) return 53;
  if (!math_test) return 54;
  if (!string_ops) return 55;
  if (!wide_ops) return 56;
  if (!unicode_ops) return 57;
  if (!run_conf2) return 58;

  if (set_point(3, 4) != 34) return 1;
  if (get_sum() != 7) return 2;
  if (set_elem(2, 9) != 910) return 3;
  if (set_text() != 55) return 4;
  if (copy_test() != 44) return 5;
  if (init_sum() != 3458) return 9;
  if (multi() != 44) return 10;
  if (run() != 21) return 11;
  if (bump_init() != 23) return 12;
  if (sum_bp() != 5) return 13;
  if (ref_test() != 15) return 14;
  if (run_conf() != 28) return 15;
  if (run_oop() != 54) return 16;
  if (math_test() != 16) return 20;
  if (string_ops() != 1111111111) return 21;
  if (wide_ops() != 111) return 22;
  if (unicode_ops() != 111111) return 23;
  if (run_conf2() != 300) return 24;
  if (tick() != 1) return 17;
  if (tick() != 2) return 18;

  if (stc_host_cycle() != 0) return 6;
  if (get_sum() != 7) return 7;
  if (tick() != 3) return 19;
  if (stc_host_shutdown() != 0) return 8;
  return 0;
}
