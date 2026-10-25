/* M4.5 advanced library harness: bindings, short-circuit, casts, retained state. */

typedef unsigned char u8;
typedef int (*int_fn)(void);
typedef int (*int1_fn)(int);
typedef int (*int2_fn)(int, int);
typedef int (*int_from_double_fn)(double);
typedef double (*double2_fn)(double, double);
typedef unsigned int (*uint2_fn)(unsigned int, unsigned int);
typedef unsigned short (*ushort2_fn)(unsigned short, unsigned short);
typedef long long (*ll1_fn)(long long);
typedef long long (*ll2_fn)(long long, long long);
typedef long long (*ll3_fn)(long long, int);
typedef long long (*size_fn)(void);
typedef int (*buf_fn)(void*);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

static u8 buffer[256];
static u8 buffer2[256];

static int read_i32(const u8* p) {
  return (int)((unsigned)p[0] | ((unsigned)p[1] << 8) | ((unsigned)p[2] << 16) |
               ((unsigned)p[3] << 24));
}

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int_fn short_circuit = (int_fn)stc_host_symbol("ShortCircuit");
  int_from_double_fn trunc_half = (int_from_double_fn)stc_host_symbol("TruncHalf");
  int1_fn set_value = (int1_fn)stc_host_symbol("SetValue");
  int_fn get_value = (int_fn)stc_host_symbol("GetValue");
  int_fn set_agg = (int_fn)stc_host_symbol("SetAgg");
  int_fn get_agg = (int_fn)stc_host_symbol("GetAgg");
  int_fn clear_agg = (int_fn)stc_host_symbol("ClearAgg");
  int1_fn bump_fb = (int1_fn)stc_host_symbol("BumpFb");
  int_fn get_fb = (int_fn)stc_host_symbol("GetFb");
  int_fn get_result = (int_fn)stc_host_symbol("GetResult");
  int2_fn pow_int = (int2_fn)stc_host_symbol("PowInt");
  double2_fn pow_real = (double2_fn)stc_host_symbol("PowReal");
  int2_fn mod_test = (int2_fn)stc_host_symbol("ModTest");
  uint2_fn u_div = (uint2_fn)stc_host_symbol("UDiv");
  ushort2_fn bit_ops = (ushort2_fn)stc_host_symbol("BitOps");
  ll2_fn time_add = (ll2_fn)stc_host_symbol("TimeAdd");
  ll1_fn date_to_ns = (ll1_fn)stc_host_symbol("DateToNs");
  ll3_fn time_scale = (ll3_fn)stc_host_symbol("TimeScale");
  int_fn loop_control = (int_fn)stc_host_symbol("LoopControl");
  int_fn get_cycles = (int_fn)stc_host_symbol("GetCycles");
  size_fn retain_size = (size_fn)stc_host_symbol("stc_retain_size");
  size_fn retain_hash = (size_fn)stc_host_symbol("stc_retain_layout_hash");
  buf_fn retain_save = (buf_fn)stc_host_symbol("stc_retain_save");
  buf_fn retain_load = (buf_fn)stc_host_symbol("stc_retain_load");
  if (!short_circuit || !trunc_half || !set_value || !get_value || !get_result ||
      !set_agg || !get_agg || !clear_agg || !bump_fb || !get_fb ||
      !pow_int || !pow_real || !mod_test || !u_div || !bit_ops || !time_add ||
      !date_to_ns || !time_scale || !loop_control || !get_cycles ||
      !retain_size || !retain_hash || !retain_save || !retain_load) {
    return 32;
  }

  if (short_circuit() != 0) return 1;
  if (trunc_half(7.0) != 3) return 2;
  if (trunc_half(-7.0) != -3) return 3;

  if (get_cycles() != 0) return 22;
  if (stc_host_cycle() != 0) return 33;
  if (get_cycles() != 1) return 23;
  if (get_result() != 10) return 4;
  if (stc_host_cycle() != 0) return 34;
  if (get_cycles() != 2) return 24; /* seed 5 + VAR_CONFIG 5 */

  if (retain_size() != 84) return 5;
  if (retain_hash() == 0) return 6;
  if (set_value(123) != 123) return 7;
  if (set_agg() != 0) return 50;
  if (get_agg() != 46) return 51;
  if (bump_fb(5) != 5) return 67;
  if (get_fb() != 7) return 68;
  if (retain_save(buffer) != 0) return 8;
  /* Program-instance leaves sort first: sp.x, sp.y, then the string. */
  if (read_i32(buffer + 0) != 2) return 52;
  if (read_i32(buffer + 4) != 2) return 53;
  if (read_i32(buffer + 8) != 1) return 54;
  if (buffer[12] != 'p') return 55;
  /* SavedFb follows Saved: CV, inc, label, then Q. */
  if (read_i32(buffer + 24) != 5) return 69;
  if (read_i32(buffer + 28) != 5) return 76;
  if (read_i32(buffer + 32) != 2) return 70;
  if (buffer[36] != 'f') return 71;
  if (buffer[37] != 'b') return 72;
  if (read_i32(buffer + 44) != 5) return 77;
  if (bump_fb(-5) != 0) return 73;
  if (get_fb() != 2) return 74;
  if (set_value(0) != 0) return 9;
  if (clear_agg() != 0) return 56;
  if (get_agg() != 0) return 57;
  if (retain_load(buffer) != 0) return 10;
  if (get_value() != 123) return 11;
  if (get_agg() != 46) return 58;
  if (get_fb() != 7) return 75;
  if (stc_host_cycle() != 0) return 36;
  if (retain_save(buffer2) != 0) return 59;
  if (read_i32(buffer2 + 0) != 3) return 60;
  if (retain_load(buffer) != 0) return 61;
  if (retain_save(buffer2) != 0) return 62;
  if (read_i32(buffer2 + 0) != 2) return 63;
  if (read_i32(buffer2 + 4) != 2) return 64;
  if (read_i32(buffer2 + 8) != 1) return 65;
  if (buffer2[12] != 'p') return 66;

  if (pow_int(2, 10) != 1024) return 12;
  if (pow_int(2, 0) != 1) return 13;
  if (pow_real(3.0, 2.0) != 9.0) return 14;
  if (mod_test(-7, 3) != -1) return 15;
  if (u_div(0xFFFFFFFFu, 2u) != 0x7FFFFFFFu) return 16;
  if (bit_ops(0x00FF, 0x0F0F) != 0x0FFF) return 17;
  if (time_add(5, 7) != 12) return 18;
  if (date_to_ns(1) != 86400000000000LL) return 19;
  if (loop_control() != 12) return 20;
  if (time_scale(5, 3) != 7) return 21; /* (5 * 3) / 2 */

  if (stc_host_shutdown() != 0) return 35;
  return 0;
}
