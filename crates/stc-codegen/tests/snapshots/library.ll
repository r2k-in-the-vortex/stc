; stc 0.1.0 - profile stc-2013-oop-utf8-v1
source_filename = "test"

; types
%stc.prog.MAIN = type { i32 }

; globals
@stc_io_base = internal global ptr null
@stc_io_len = internal global i64 0
@stc_metadata_json_data = private constant [1249 x i8] c"{\22abi_version\22:1,\22artifact\22:\22test\22,\22compiler\22:\22stc 0.1.0\22,\22configuration\22:\22C\22,\22exports\22:[{\22function\22:\22stc_fn_TESTADD_9731bc2064eeb366\22,\22no_mangle\22:false,\22symbol\22:\22stc_export_TESTADD\22}],\22io\22:{\22backend\22:\22null\22,\22bindings\22:[],\22image\22:{\22inputs_bytes\22:0,\22markers_bytes\22:0,\22outputs_bytes\22:0},\22input_defaults_hex\22:\22\22,\22output_defaults_hex\22:\22\22},\22mode\22:\22library\22,\22profile\22:\22stc-2013-oop-utf8-v1\22,\22programs\22:[{\22name\22:\22inst\22,\22program\22:\22Main\22,\22resource\22:\22R\22,\22task\22:null}],\22retain\22:{\22layout_hash\22:\220xcbf29ce484222325\22,\22leaves\22:[],\22size\22:0},\22spans\22:{\22entries\22:[{\22end\22:112,\22file\22:0,\22id\22:0,\22start\22:1},{\22end\22:51,\22file\22:0,\22id\22:1,\22start\22:50},{\22end\22:63,\22file\22:0,\22id\22:2,\22start\22:62},{\22end\22:89,\22file\22:0,\22id\22:3,\22start\22:82},{\22end\22:99,\22file\22:0,\22id\22:4,\22start\22:82},{\22end\22:94,\22file\22:0,\22id\22:5,\22start\22:93},{\22end\22:96,\22file\22:0,\22id\22:6,\22start\22:95},{\22end\22:98,\22file\22:0,\22id\22:7,\22start\22:97},{\22end\22:184,\22file\22:0,\22id\22:8,\22start\22:114},{\22end\22:134,\22file\22:0,\22id\22:9,\22start\22:133},{\22end\22:154,\22file\22:0,\22id\22:10,\22start\22:153},{\22end\22:172,\22file\22:0,\22id\22:11,\22start\22:153},{\22end\22:171,\22file\22:0,\22id\22:12,\22start\22:158},{\22end\22:167,\22file\22:0,\22id\22:13,\22start\22:166},{\22end\22:170,\22file\22:0,\22id\22:14,\22start\22:169},{\22end\22:273,\22file\22:0,\22id\22:15,\22start\22:186},{\22end\22:241,\22file\22:0,\22id\22:16,\22start\22:222}],\22files\22:[\22<test>\22]},\22tasks\22:[]}\00", align 1
@stc_pi_INST = internal global %stc.prog.MAIN zeroinitializer
@stc_retain_buffer = internal global [1 x i8] zeroinitializer

; declarations
declare { i32, i1 } @llvm.sadd.with.overflow.i32(i32, i32)
declare i64 @stc_rt_cycle_now_ns()
declare i32 @stc_rt_execute(i32, ptr, ptr, ptr, ptr)
declare void @stc_rt_io_configure(i64, i64, i64)
declare void @stc_rt_io_image(ptr, ptr)
declare i64 @stc_rt_monotonic_now_ns()
declare void @stc_rt_set_cycle_time_ns(i64)
declare i32 @stc_rt_state_load(ptr, i64, i64)
declare i32 @stc_rt_state_save(ptr, i64, i64)
declare void @stc_rt_trap(i32, i32)

; functions
define void @stc_cycle() {
bb0:
  call void @stc_prog_MAIN_body(ptr @stc_pi_INST)
  ret void
}

define i32 @stc_export_TESTADD(i32 %a0, i32 %a1) {
bb0:
  %t0 = call i32 @stc_fn_TESTADD_9731bc2064eeb366(i32 %a0, i32 %a1)
  ret i32 %t0
}

define internal i32 @stc_fn_TESTADD_9731bc2064eeb366(i32 %a0, i32 %a1) {
bb0:
  %t0 = alloca i32
  store i32 %a0, ptr %t0
  %t1 = alloca i32
  store i32 %a1, ptr %t1
  %t2 = alloca i32
  store i32 0, ptr %t2
  %t3 = load i32, ptr %t0
  %t4 = load i32, ptr %t1
  %t5 = call { i32, i1 } @llvm.sadd.with.overflow.i32(i32 %t3, i32 %t4)
  %t6 = extractvalue { i32, i1 } %t5, 0
  %t7 = extractvalue { i32, i1 } %t5, 1
  br i1 %t7, label %bb3, label %bb2
bb3:
  call void @stc_rt_trap(i32 1, i32 6)
  unreachable
bb2:
  store i32 %t6, ptr %t2
  br label %bb1
bb1:
  %t8 = load i32, ptr %t2
  ret i32 %t8
}

define void @stc_init() {
bb0:
  call void @stc_rt_io_configure(i64 0, i64 0, i64 0)
  call void @stc_rt_io_image(ptr @stc_io_base, ptr @stc_io_len)
  %t0 = call i32 @stc_rt_state_load(ptr @stc_retain_buffer, i64 0, i64 -3750763034362895579)
  %t1 = icmp eq i32 %t0, 0
  br i1 %t1, label %bb2, label %bb3
bb2:
  %t2 = call i32 @stc_retain_load(ptr @stc_retain_buffer)
  br label %bb3
bb3:
  ret void
}

define ptr @stc_metadata_json() {
bb0:
  ret ptr @stc_metadata_json_data
}

define internal void @stc_prog_MAIN_body(ptr %this) {
bb0:
  %t0 = getelementptr inbounds %stc.prog.MAIN, ptr %this, i32 0, i32 0
  %t1 = load i32, ptr %t0
  %t2 = call i32 @stc_fn_TESTADD_9731bc2064eeb366(i32 %t1, i32 1)
  %t3 = getelementptr inbounds %stc.prog.MAIN, ptr %this, i32 0, i32 0
  store i32 %t2, ptr %t3
  br label %bb1
bb1:
  ret void
}

define i64 @stc_retain_layout_hash() {
bb0:
  ret i64 -3750763034362895579
}

define i32 @stc_retain_load(ptr %a0) {
bb0:
  ret i32 0
}

define i32 @stc_retain_save(ptr %a0) {
bb0:
  ret i32 0
}

define i64 @stc_retain_size() {
bb0:
  ret i64 0
}

define void @stc_shutdown() {
bb0:
  %t0 = call i32 @stc_retain_save(ptr @stc_retain_buffer)
  %t1 = call i32 @stc_rt_state_save(ptr @stc_retain_buffer, i64 0, i64 -3750763034362895579)
  ret void
}

