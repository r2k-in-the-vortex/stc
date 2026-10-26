; stc 0.1.0 - profile stc-2013-oop-utf8-v1
source_filename = "test"
target triple = "x86_64-unknown-linux-gnu"

; globals
@stc_metadata_json_data = private constant [1665 x i8] c"{\22abi_version\22:2,\22artifact\22:\22test\22,\22build_id\22:\220x6299e204cd4ea6d6e11e6599cc4c2228\22,\22compiler\22:\22stc 0.1.0\22,\22configuration\22:\22C\22,\22descriptor_digest\22:\220xf2aee9334fc4b38b0ed5b17d059215e9aaca440165a893821c494d35b53d8e58\22,\22exports\22:[{\22function\22:\22stc_fn__ROOT_n_TESTADD_9731bc2064eeb366\22,\22no_mangle\22:false,\22symbol\22:\22stc_export_TESTADD\22}],\22io\22:{\22backend\22:\22null\22,\22bindings\22:[],\22devices\22:[],\22file\22:null,\22image\22:{\22inputs_bytes\22:0,\22markers_bytes\22:0,\22outputs_bytes\22:0},\22input_defaults_hex\22:\22\22,\22output_defaults_hex\22:\22\22,\22var_access\22:[]},\22manifest\22:{\22digest\22:\220x4bb89281d7be595c8df2235be63d49f24eac3991a55bc5270218d50eb0595d26\22,\22format\22:1},\22mode\22:\22library\22,\22module\22:{\22file\22:\22\22,\22format\22:1},\22profile\22:\22stc-2013-oop-utf8-v1\22,\22programs\22:[{\22name\22:\22inst\22,\22program\22:\22Main\22,\22resource\22:\22R\22,\22task\22:null}],\22retain\22:{\22enabled\22:true,\22file\22:\22test.stcstate\22,\22layout_hash\22:\220xfeee76fe0301e686\22,\22leaves\22:[],\22on_mismatch\22:\22error\22,\22size\22:0},\22spans\22:{\22entries\22:[{\22end\22:112,\22file\22:0,\22id\22:0,\22start\22:1},{\22end\22:51,\22file\22:0,\22id\22:1,\22start\22:50},{\22end\22:63,\22file\22:0,\22id\22:2,\22start\22:62},{\22end\22:89,\22file\22:0,\22id\22:3,\22start\22:82},{\22end\22:99,\22file\22:0,\22id\22:4,\22start\22:82},{\22end\22:94,\22file\22:0,\22id\22:5,\22start\22:93},{\22end\22:96,\22file\22:0,\22id\22:6,\22start\22:95},{\22end\22:98,\22file\22:0,\22id\22:7,\22start\22:97},{\22end\22:184,\22file\22:0,\22id\22:8,\22start\22:114},{\22end\22:134,\22file\22:0,\22id\22:9,\22start\22:133},{\22end\22:154,\22file\22:0,\22id\22:10,\22start\22:153},{\22end\22:172,\22file\22:0,\22id\22:11,\22start\22:153},{\22end\22:171,\22file\22:0,\22id\22:12,\22start\22:158},{\22end\22:167,\22file\22:0,\22id\22:13,\22start\22:166},{\22end\22:170,\22file\22:0,\22id\22:14,\22start\22:169},{\22end\22:273,\22file\22:0,\22id\22:15,\22start\22:186},{\22end\22:241,\22file\22:0,\22id\22:16,\22start\22:222}],\22files\22:[\22<test>\22]},\22target\22:\22x86_64-unknown-linux-gnu\22,\22tasks\22:[]}\00", align 1
@stc_module_api = internal global ptr null
@stc_module_descriptor_data = private constant [507 x i8] c"STCMOD\00\00\01\00\00\00\FB\01\00\00\0F\00\00\00\00\00\00\00\02\00\01\00\01\00\00\08\14\00\00\00stc-2013-oop-utf8-v1\09\00\00\00stc 0.1.0\18\00\00\00x86_64-unknown-linux-gnub\99\E2\04\CDN\A6\D6\E1\1Ee\99\CCL\22(K\B8\92\81\D7\BEY\5C\8D\F2#[\E6=I\F2N\AC9\91\A5[\C5'\02\18\D5\0E\B0Y]&\01\00\13\00\00\00\02\00\00\00\09\00\00\00\98z\8E\F0\81;%\98\EC\04>\BD\D6tK\08\19\00\00\00_ROOT::MAIN#BODY@()->VOID\08\00\00\00()->VOIDB\00\00\00\AF\05B\02\B1\DE\DF\8B~b\D3\F7\227\92|&\00\00\00_ROOT::TESTADD@(IN:DINT,IN:DINT)->DINT\17\00\00\00(IN:DINT,IN:DINT)->DINTA\00\00\00\00\00\00\00\01\00\18\B7\A5\CC\01\9E&!ci*K\FA\FEa\9A\01\00\00\00\01\00\8C\10#C\9C\EA\ACZ\D9T\0Ck\F9\A2\A0\A8\02\00\00\00\02\00\98z\8E\F0\81;%\98\EC\04>\BD\D6tK\08\03\00\00\00\02\00\AF\05B\02\B1\DE\DF\8B~b\D3\F7\227\92|\04\00\00\00\05\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\05\00\00\00\05\07\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\06\00\00\00\05\08\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\07\00\00\00\05\09\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\08\00\00\00\05\0A\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00", align 8
@stc_module_function_table = internal constant [2 x ptr] [ptr @stc_prog__ROOT_n_MAIN_body, ptr @stc_fn__ROOT_n_TESTADD_9731bc2064eeb366], align 8
@stc_module_slots = internal global ptr null
@stc_retain_buffer = internal global [1 x i8] zeroinitializer

; declarations
declare { i32, i1 } @llvm.sadd.with.overflow.i32(i32, i32)

; functions
define dso_local dllexport i32 @stc_export_TESTADD(i32 %a0, i32 %a1) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp eq ptr %t0, null
  br i1 %t1, label %bb3, label %bb2
bb3:
  ret i32 0
bb2:
  %t2 = getelementptr ptr, ptr %t0, i64 3
  %t3 = load ptr, ptr %t2, align 8
  %t4 = load ptr, ptr %t3, align 8
  %t5 = call i32 %t4(i32 %a0, i32 %a1, ptr %t0)
  ret i32 %t5
}

define internal i32 @stc_fn__ROOT_n_TESTADD_9731bc2064eeb366(i32 %a0, i32 %a1, ptr %slots) uwtable {
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
  %t8 = getelementptr ptr, ptr %slots, i64 4
  %t9 = load ptr, ptr %t8, align 8
  %t10 = load ptr, ptr %t9, align 8
  call void %t10(i32 1, i32 6)
  unreachable
bb2:
  store i32 %t6, ptr %t2
  br label %bb1
bb1:
  %t11 = load i32, ptr %t2
  ret i32 %t11
}

define dso_local dllexport ptr @stc_metadata_json() uwtable {
bb0:
  ret ptr @stc_metadata_json_data
}

define dso_local dllexport i32 @stc_module_attach(ptr %api, ptr %slots, i32 %slot_count, i64 %generation) uwtable {
bb0:
  %t0 = icmp eq i64 %generation, 0
  br i1 %t0, label %bb1, label %bb2
bb1:
  ret i32 1
bb2:
  %t1 = icmp eq ptr %api, null
  %t2 = icmp eq ptr %slots, null
  %t3 = or i1 %t1, %t2
  br i1 %t3, label %bb3, label %bb4
bb3:
  ret i32 1
bb4:
  %t4 = getelementptr i8, ptr %api, i64 0
  %t5 = load i32, ptr %t4, align 4
  %t6 = lshr i32 %t5, 16
  %t7 = icmp ne i32 %t6, 2
  br i1 %t7, label %bb5, label %bb6
bb5:
  ret i32 2
bb6:
  %t8 = getelementptr i8, ptr %api, i64 4
  %t9 = load i32, ptr %t8, align 4
  %t10 = icmp ult i32 %t9, 96
  br i1 %t10, label %bb7, label %bb8
bb7:
  ret i32 2
bb8:
  %t11 = icmp ne i32 %slot_count, 9
  br i1 %t11, label %bb9, label %bb10
bb9:
  ret i32 3
bb10:
  store ptr %api, ptr @stc_module_api, align 8
  store ptr %slots, ptr @stc_module_slots, align 8
  ret i32 0
}

define dso_local dllexport i32 @stc_module_cycle(i64 %generation) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp eq i64 %generation, 0
  br i1 %t1, label %bb2, label %bb3
bb2:
  ret i32 1
bb3:
  %t2 = icmp eq ptr %t0, null
  br i1 %t2, label %bb4, label %bb5
bb4:
  ret i32 1
bb5:
  %t3 = getelementptr ptr, ptr %t0, i64 1
  %t4 = load ptr, ptr %t3, align 8
  %t5 = load ptr, ptr %t4, align 8
  %t6 = alloca [1 x ptr]
  %t7 = getelementptr inbounds [1 x ptr], ptr %t6, i64 0, i64 0
  store ptr %t5, ptr %t7, align 8
  %t8 = getelementptr ptr, ptr %t0, i64 2
  %t9 = load ptr, ptr %t8, align 8
  %t10 = load ptr, ptr %t9, align 8
  call void %t10(ptr %t6, ptr %t0)
  ret i32 0
}

define dso_local dllexport ptr @stc_module_descriptor(ptr %out_size) uwtable {
bb0:
  store i64 507, ptr %out_size, align 8
  ret ptr @stc_module_descriptor_data
}

define dso_local dllexport void @stc_module_detach(i64 %generation) uwtable {
bb0:
  store ptr null, ptr @stc_module_api, align 8
  store ptr null, ptr @stc_module_slots, align 8
  ret void
}

define dso_local dllexport ptr @stc_module_functions(ptr %out_count) uwtable {
bb0:
  store i32 2, ptr %out_count, align 4
  ret ptr @stc_module_function_table
}

define dso_local dllexport i32 @stc_module_prepare(i32 %mode, i64 %generation, ptr %mask, i32 %mask_size) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp eq i64 %generation, 0
  br i1 %t1, label %bb2, label %bb3
bb2:
  ret i32 1
bb3:
  %t2 = icmp eq ptr %t0, null
  br i1 %t2, label %bb4, label %bb5
bb4:
  ret i32 1
bb5:
  %t3 = icmp ugt i32 %mode, 1
  br i1 %t3, label %bb6, label %bb7
bb6:
  ret i32 1
bb7:
  %t4 = icmp eq i32 %mode, 1
  %t5 = icmp ne i32 %mask_size, 2
  %t6 = and i1 %t4, %t5
  br i1 %t6, label %bb8, label %bb9
bb8:
  ret i32 1
bb9:
  %t7 = icmp eq i32 %mode, 0
  br i1 %t7, label %bb10, label %bb11
bb10:
  %t8 = getelementptr ptr, ptr %t0, i64 7
  %t9 = load ptr, ptr %t8, align 8
  %t10 = load ptr, ptr %t9, align 8
  %t11 = call i32 %t10(ptr @stc_retain_buffer, i64 0, i64 -76993310274492794)
  %t12 = icmp eq i32 %t11, 0
  br i1 %t12, label %bb12, label %bb13
bb12:
  %t13 = call i32 @stc_retain_load(ptr @stc_retain_buffer)
  br label %bb13
bb13:
  br label %bb11
bb11:
  ret i32 0
}

define dso_local dllexport i32 @stc_module_shutdown(i32 %mode) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp ugt i32 %mode, 1
  br i1 %t1, label %bb2, label %bb3
bb2:
  ret i32 1
bb3:
  %t2 = icmp eq ptr %t0, null
  br i1 %t2, label %bb4, label %bb5
bb4:
  ret i32 1
bb5:
  %t3 = call i32 @stc_retain_save(ptr @stc_retain_buffer)
  %t4 = getelementptr ptr, ptr %t0, i64 8
  %t5 = load ptr, ptr %t4, align 8
  %t6 = load ptr, ptr %t5, align 8
  %t7 = call i32 %t6(ptr @stc_retain_buffer, i64 0, i64 -76993310274492794)
  ret i32 0
}

define internal void @stc_prog__ROOT_n_MAIN_body(ptr %fields, ptr %slots) uwtable {
bb0:
  %t0 = getelementptr inbounds [1 x ptr], ptr %fields, i64 0, i64 0
  %t1 = load ptr, ptr %t0, align 8
  %t2 = load i32, ptr %t1
  %t3 = getelementptr ptr, ptr %slots, i64 3
  %t4 = load ptr, ptr %t3, align 8
  %t5 = load ptr, ptr %t4, align 8
  %t6 = call i32 %t5(i32 %t2, i32 1, ptr %slots)
  %t7 = getelementptr inbounds [1 x ptr], ptr %fields, i64 0, i64 0
  %t8 = load ptr, ptr %t7, align 8
  store i32 %t6, ptr %t8
  br label %bb1
bb1:
  ret void
}

define dso_local dllexport i64 @stc_retain_layout_hash() uwtable {
bb0:
  ret i64 -76993310274492794
}

define dso_local dllexport i32 @stc_retain_load(ptr %a0) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp eq ptr %t0, null
  br i1 %t1, label %bb2, label %bb3
bb2:
  ret i32 1
bb3:
  ret i32 0
}

define dso_local dllexport i32 @stc_retain_save(ptr %a0) uwtable {
bb0:
  %t0 = load ptr, ptr @stc_module_slots, align 8
  %t1 = icmp eq ptr %t0, null
  br i1 %t1, label %bb2, label %bb3
bb2:
  ret i32 1
bb3:
  ret i32 0
}

define dso_local dllexport i64 @stc_retain_size() uwtable {
bb0:
  ret i64 0
}

