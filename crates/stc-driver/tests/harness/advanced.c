/* M4 advanced library harness: bindings, short-circuit, casts, retained state. */

typedef unsigned char u8;

extern void stc_init(void);
extern void stc_cycle(void);
extern void stc_shutdown(void);
extern int ShortCircuit(void);
extern int TruncHalf(double x);
extern int SetValue(int v);
extern int GetValue(void);
extern int GetResult(void);
extern int PowInt(int a, int b);
extern double PowReal(double a, double b);
extern int ModTest(int a, int b);
extern unsigned int UDiv(unsigned int a, unsigned int b);
extern unsigned short BitOps(unsigned short a, unsigned short b);
extern long long TimeAdd(long long a, long long b);
extern long long DateToNs(long long days);
extern long long TimeScale(long long t, int f);
extern int LoopControl(void);
extern int GetCycles(void);
extern long long stc_retain_size(void);
extern long long stc_retain_layout_hash(void);
extern int stc_retain_save(void* buffer);
extern int stc_retain_load(const void* buffer);

static u8 buffer[256];

int main(void) {
  if (ShortCircuit() != 0) return 1;
  if (TruncHalf(7.0) != 3) return 2;
  if (TruncHalf(-7.0) != -3) return 3;

  stc_init();
  if (GetCycles() != 0) return 22;
  stc_cycle();
  if (GetCycles() != 1) return 23;
  if (GetResult() != 10) return 4;
  stc_cycle();
  if (GetCycles() != 2) return 24; /* seed 5 + VAR_CONFIG 5 */

  if (stc_retain_size() != 4) return 5;
  if (stc_retain_layout_hash() == 0) return 6;
  if (SetValue(123) != 123) return 7;
  if (stc_retain_save(buffer) != 0) return 8;
  if (SetValue(0) != 0) return 9;
  if (stc_retain_load(buffer) != 0) return 10;
  if (GetValue() != 123) return 11;

  if (PowInt(2, 10) != 1024) return 12;
  if (PowInt(2, 0) != 1) return 13;
  if (PowReal(3.0, 2.0) != 9.0) return 14;
  if (ModTest(-7, 3) != -1) return 15;
  if (UDiv(0xFFFFFFFFu, 2u) != 0x7FFFFFFFu) return 16;
  if (BitOps(0x00FF, 0x0F0F) != 0x0FFF) return 17;
  if (TimeAdd(5, 7) != 12) return 18;
  if (DateToNs(1) != 86400000000000LL) return 19;
  if (LoopControl() != 12) return 20;
  if (TimeScale(5, 3) != 7) return 21; /* (5 * 3) / 2 */

  stc_shutdown();
  return 0;
}
