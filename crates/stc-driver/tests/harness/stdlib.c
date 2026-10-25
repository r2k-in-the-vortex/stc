/* M5 standard function-block harness: timers, edges, counters, bistables. */

typedef unsigned char u8;

typedef unsigned char (*step_fn)(int, u8, u8);
typedef long long (*et_fn)(int);
typedef short (*cv_fn)(int);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);
extern void stc_rt_set_cycle_time_ns(long long ns);

static int at(long long ns) {
  stc_rt_set_cycle_time_ns(ns);
  return stc_host_cycle();
}

#define REQUIRE(condition, code) \
  do {                           \
    if (!(condition)) return code; \
  } while (0)

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  step_fn step = (step_fn)stc_host_symbol("FbStep");
  et_fn et = (et_fn)stc_host_symbol("FbET");
  cv_fn cv = (cv_fn)stc_host_symbol("FbCV");
  if (!step) return 32;
  if (!et) return 33;
  if (!cv) return 34;

  /* TON: start, ET rises, Q latches at PT, IN low resets. */
  REQUIRE(at(0) == 0, 40);
  REQUIRE(step(0, 1, 0) == 0, 1);
  REQUIRE(et(0) == 0, 2);
  REQUIRE(at(50) == 0, 40);
  REQUIRE(step(0, 1, 0) == 0, 3);
  REQUIRE(et(0) == 50, 4);
  REQUIRE(at(150) == 0, 40);
  REQUIRE(step(0, 1, 0) == 1, 5);
  REQUIRE(et(0) == 100, 6);
  REQUIRE(step(0, 1, 0) == 1, 7);
  REQUIRE(et(0) == 100, 8);
  REQUIRE(step(0, 0, 0) == 0, 9);
  REQUIRE(et(0) == 0, 10);
  REQUIRE(at(200) == 0, 40);
  REQUIRE(step(0, 1, 0) == 0, 11);
  REQUIRE(at(300) == 0, 40);
  REQUIRE(step(0, 1, 0) == 1, 12);
  REQUIRE(et(0) == 100, 13);

  /* TOF: Q stays high for PT after IN falls. */
  REQUIRE(at(1000) == 0, 40);
  REQUIRE(step(1, 1, 0) == 1, 14);
  REQUIRE(et(1) == 0, 15);
  REQUIRE(at(1050) == 0, 40);
  REQUIRE(step(1, 0, 0) == 1, 16);
  REQUIRE(et(1) == 0, 17);
  REQUIRE(at(1100) == 0, 40);
  REQUIRE(step(1, 0, 0) == 1, 18);
  REQUIRE(et(1) == 50, 19);
  REQUIRE(at(1150) == 0, 40);
  REQUIRE(step(1, 0, 0) == 0, 20);
  REQUIRE(et(1) == 100, 21);
  REQUIRE(at(1200) == 0, 40);
  REQUIRE(step(1, 1, 0) == 1, 22);
  REQUIRE(et(1) == 0, 23);

  /* TP: fixed pulse; retrigger requires IN low. */
  REQUIRE(at(2000) == 0, 40);
  REQUIRE(step(2, 1, 0) == 1, 24);
  REQUIRE(et(2) == 0, 25);
  REQUIRE(at(2050) == 0, 40);
  REQUIRE(step(2, 1, 0) == 1, 26);
  REQUIRE(et(2) == 50, 27);
  REQUIRE(at(2150) == 0, 40);
  REQUIRE(step(2, 1, 0) == 0, 28);
  REQUIRE(et(2) == 100, 29);
  REQUIRE(at(2200) == 0, 40);
  REQUIRE(step(2, 1, 0) == 0, 30);
  REQUIRE(et(2) == 100, 31);
  REQUIRE(step(2, 0, 0) == 0, 32);
  REQUIRE(et(2) == 0, 33);
  REQUIRE(at(2260) == 0, 40);
  REQUIRE(step(2, 1, 0) == 1, 34);
  REQUIRE(at(2300) == 0, 40);
  REQUIRE(step(2, 1, 0) == 1, 35);
  REQUIRE(et(2) == 40, 36);
  REQUIRE(at(2400) == 0, 40);
  REQUIRE(step(2, 1, 0) == 0, 37);
  REQUIRE(et(2) == 100, 38);
  REQUIRE(step(2, 0, 0) == 0, 39);

  /* Rising and falling edge triggers. */
  REQUIRE(step(3, 0, 0) == 0, 50);
  REQUIRE(step(3, 1, 0) == 1, 51);
  REQUIRE(step(3, 1, 0) == 0, 52);
  REQUIRE(step(3, 0, 0) == 0, 53);
  REQUIRE(step(3, 1, 0) == 1, 54);
  REQUIRE(step(4, 0, 0) == 0, 55);
  REQUIRE(step(4, 1, 0) == 0, 56);
  REQUIRE(step(4, 0, 0) == 1, 57);
  REQUIRE(step(4, 0, 0) == 0, 58);
  REQUIRE(step(4, 1, 0) == 0, 59);
  REQUIRE(step(4, 0, 0) == 1, 60);

  /* CTU: rising edges count, R clears, Q at PV. */
  REQUIRE(step(5, 0, 0) == 0, 61);
  REQUIRE(cv(5) == 0, 62);
  REQUIRE(step(5, 1, 0) == 0, 63);
  REQUIRE(cv(5) == 1, 64);
  REQUIRE(step(5, 1, 0) == 0, 65);
  REQUIRE(cv(5) == 1, 66);
  REQUIRE(step(5, 0, 0) == 0, 67);
  REQUIRE(step(5, 1, 0) == 0, 68);
  REQUIRE(cv(5) == 2, 69);
  REQUIRE(step(5, 0, 0) == 0, 70);
  REQUIRE(step(5, 1, 0) == 1, 71);
  REQUIRE(cv(5) == 3, 72);
  REQUIRE(step(5, 0, 1) == 0, 73);
  REQUIRE(cv(5) == 0, 74);
  REQUIRE(step(5, 1, 0) == 0, 75);
  REQUIRE(cv(5) == 1, 76);

  /* CTD: LD loads PV, falling edges count down. */
  REQUIRE(step(6, 0, 1) == 0, 80);
  REQUIRE(cv(6) == 3, 81);
  REQUIRE(step(6, 1, 0) == 0, 82);
  REQUIRE(cv(6) == 2, 83);
  REQUIRE(step(6, 0, 0) == 0, 84);
  REQUIRE(step(6, 1, 0) == 0, 85);
  REQUIRE(cv(6) == 1, 86);
  REQUIRE(step(6, 0, 0) == 0, 87);
  REQUIRE(step(6, 1, 0) == 1, 88);
  REQUIRE(cv(6) == 0, 89);
  REQUIRE(step(6, 0, 1) == 0, 90);
  REQUIRE(cv(6) == 3, 91);

  /* CTUD: up and down edges on one instance, R clears. */
  REQUIRE(step(7, 0, 0) == 0, 100);
  REQUIRE(cv(7) == 0, 101);
  REQUIRE(step(7, 1, 0) == 0, 102);
  REQUIRE(cv(7) == 1, 103);
  REQUIRE(step(7, 0, 0) == 0, 104);
  REQUIRE(step(7, 1, 0) == 0, 105);
  REQUIRE(cv(7) == 2, 106);
  REQUIRE(step(7, 0, 0) == 0, 107);
  REQUIRE(step(7, 1, 0) == 1, 108);
  REQUIRE(cv(7) == 3, 109);
  REQUIRE(step(7, 0, 1) == 0, 110);
  REQUIRE(cv(7) == 0, 111);
  REQUIRE(step(10, 0, 0) == 1, 112);
  REQUIRE(step(7, 1, 0) == 0, 113);
  REQUIRE(step(7, 0, 0) == 0, 114);
  REQUIRE(step(7, 1, 0) == 0, 115);
  REQUIRE(step(7, 0, 0) == 0, 116);
  REQUIRE(step(7, 1, 0) == 1, 117);
  REQUIRE(cv(7) == 3, 118);
  REQUIRE(step(10, 1, 0) == 0, 119);
  REQUIRE(cv(7) == 2, 120);
  REQUIRE(step(10, 0, 0) == 0, 121);
  REQUIRE(step(10, 1, 0) == 0, 122);
  REQUIRE(cv(7) == 1, 123);
  REQUIRE(step(10, 0, 0) == 0, 124);
  REQUIRE(step(10, 1, 0) == 1, 125);
  REQUIRE(cv(7) == 0, 126);
  REQUIRE(step(7, 0, 1) == 0, 127);
  REQUIRE(cv(7) == 0, 128);

  /* SR is set-dominant and retains when idle. */
  REQUIRE(step(8, 0, 0) == 0, 130);
  REQUIRE(step(8, 1, 0) == 1, 131);
  REQUIRE(step(8, 0, 0) == 1, 132);
  REQUIRE(step(8, 1, 1) == 1, 133);
  REQUIRE(step(8, 0, 1) == 0, 134);
  REQUIRE(step(8, 0, 0) == 0, 135);

  /* RS is reset-dominant and retains when idle. */
  REQUIRE(step(9, 0, 0) == 0, 140);
  REQUIRE(step(9, 1, 0) == 1, 141);
  REQUIRE(step(9, 0, 0) == 1, 142);
  REQUIRE(step(9, 1, 1) == 0, 143);
  REQUIRE(step(9, 1, 0) == 1, 144);
  REQUIRE(step(9, 0, 1) == 0, 145);
  REQUIRE(step(9, 0, 0) == 0, 146);

  if (stc_host_shutdown() != 0) return 41;
  return 0;
}
