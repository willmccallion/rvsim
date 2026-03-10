# Torture test -- seed 10003, length 200
#include "riscv_test.h"
#include "test_macros.h"

RVTEST_RV64U
RVTEST_CODE_BEGIN

  # Initialize registers
  li x5, 0x546be46ca457238e
  li x6, 0x17125d5a41de2438
  li x7, 0x35f0481f4158fc51
  li x8, 0x428c147ccdc72f84
  li x9, 0x6e7b91a24d6d7e87
  li x10, 0x791b1b3ac5f64383
  li x11, 0x061f9b6ab63342b1
  li x12, 0x71efc901d7dde8cb
  li x13, 0x0166a45036b22808
  li x14, 0x5d03dc5dcbe19c58
  li x15, 0x07ef41576793f7b7
  li x16, 0x6fc336bf28f4a2d9
  li x17, 0x084a46f57b75787a
  li x18, 0x1df5779d4c8073bf
  li x19, 0x0e67db23dfef0075
  li x20, 0x428fa10c57b4a989
  li x21, 0x1faee12555dc736e
  li x22, 0x28214d72d367f0cc
  li x23, 0x58a351b5ea75bb2a
  li x24, 0x712c5f6f33cf95f7
  li x25, 0x10b7d2737cc42167
  li x26, 0x2919ec6c4b0552ae
  li x27, 0x2435f677f3090f42
  li x28, 0x38e332e3de39ef80
  li x29, 0x71509e2812b7f041
  li x30, 0x64198b872399fc2a
  li x31, 0x010ebcbf5781ad9d

  # Set up memory scratch base
  la x4, scratch_data

  # Initialize scratch memory
  li x5, 0xCAFEBABEDEADBEEF
  sd x5, 0(x4)
  sd x5, 8(x4)
  sd x5, 16(x4)
  sd x5, 24(x4)
  sd x5, 32(x4)
  sd x5, 40(x4)
  sd x5, 48(x4)
  sd x5, 56(x4)
  sd x5, 64(x4)
  sd x5, 72(x4)
  sd x5, 80(x4)
  sd x5, 88(x4)
  sd x5, 96(x4)
  sd x5, 104(x4)
  sd x5, 112(x4)
  sd x5, 120(x4)
  sd x5, 128(x4)
  sd x5, 136(x4)
  sd x5, 144(x4)
  sd x5, 152(x4)
  sd x5, 160(x4)
  sd x5, 168(x4)
  sd x5, 176(x4)
  sd x5, 184(x4)
  sd x5, 192(x4)
  sd x5, 200(x4)
  sd x5, 208(x4)
  sd x5, 216(x4)
  sd x5, 224(x4)
  sd x5, 232(x4)
  sd x5, 240(x4)
  sd x5, 248(x4)
  sd x5, 256(x4)
  sd x5, 264(x4)
  sd x5, 272(x4)
  sd x5, 280(x4)
  sd x5, 288(x4)
  sd x5, 296(x4)
  sd x5, 304(x4)
  sd x5, 312(x4)
  sd x5, 320(x4)
  sd x5, 328(x4)
  sd x5, 336(x4)
  sd x5, 344(x4)
  sd x5, 352(x4)
  sd x5, 360(x4)
  sd x5, 368(x4)
  sd x5, 376(x4)
  sd x5, 384(x4)
  sd x5, 392(x4)
  sd x5, 400(x4)
  sd x5, 408(x4)
  sd x5, 416(x4)
  sd x5, 424(x4)
  sd x5, 432(x4)
  sd x5, 440(x4)
  sd x5, 448(x4)
  sd x5, 456(x4)
  sd x5, 464(x4)
  sd x5, 472(x4)
  sd x5, 480(x4)
  sd x5, 488(x4)
  sd x5, 496(x4)
  sd x5, 504(x4)
  sd x5, 512(x4)
  sd x5, 520(x4)
  sd x5, 528(x4)
  sd x5, 536(x4)
  sd x5, 544(x4)
  sd x5, 552(x4)
  sd x5, 560(x4)
  sd x5, 568(x4)
  sd x5, 576(x4)
  sd x5, 584(x4)
  sd x5, 592(x4)
  sd x5, 600(x4)
  sd x5, 608(x4)
  sd x5, 616(x4)
  sd x5, 624(x4)
  sd x5, 632(x4)
  sd x5, 640(x4)
  sd x5, 648(x4)
  sd x5, 656(x4)
  sd x5, 664(x4)
  sd x5, 672(x4)
  sd x5, 680(x4)
  sd x5, 688(x4)
  sd x5, 696(x4)
  sd x5, 704(x4)
  sd x5, 712(x4)
  sd x5, 720(x4)
  sd x5, 728(x4)
  sd x5, 736(x4)
  sd x5, 744(x4)
  sd x5, 752(x4)
  sd x5, 760(x4)
  sd x5, 768(x4)
  sd x5, 776(x4)
  sd x5, 784(x4)
  sd x5, 792(x4)
  sd x5, 800(x4)
  sd x5, 808(x4)
  sd x5, 816(x4)
  sd x5, 824(x4)
  sd x5, 832(x4)
  sd x5, 840(x4)
  sd x5, 848(x4)
  sd x5, 856(x4)
  sd x5, 864(x4)
  sd x5, 872(x4)
  sd x5, 880(x4)
  sd x5, 888(x4)
  sd x5, 896(x4)
  sd x5, 904(x4)
  sd x5, 912(x4)
  sd x5, 920(x4)
  sd x5, 928(x4)
  sd x5, 936(x4)
  sd x5, 944(x4)
  sd x5, 952(x4)
  sd x5, 960(x4)
  sd x5, 968(x4)
  sd x5, 976(x4)
  sd x5, 984(x4)
  sd x5, 992(x4)
  sd x5, 1000(x4)
  sd x5, 1008(x4)
  sd x5, 1016(x4)
  sd x5, 1024(x4)
  sd x5, 1032(x4)
  sd x5, 1040(x4)
  sd x5, 1048(x4)
  sd x5, 1056(x4)
  sd x5, 1064(x4)
  sd x5, 1072(x4)
  sd x5, 1080(x4)
  sd x5, 1088(x4)
  sd x5, 1096(x4)
  sd x5, 1104(x4)
  sd x5, 1112(x4)
  sd x5, 1120(x4)
  sd x5, 1128(x4)
  sd x5, 1136(x4)
  sd x5, 1144(x4)
  sd x5, 1152(x4)
  sd x5, 1160(x4)
  sd x5, 1168(x4)
  sd x5, 1176(x4)
  sd x5, 1184(x4)
  sd x5, 1192(x4)
  sd x5, 1200(x4)
  sd x5, 1208(x4)
  sd x5, 1216(x4)
  sd x5, 1224(x4)
  sd x5, 1232(x4)
  sd x5, 1240(x4)
  sd x5, 1248(x4)
  sd x5, 1256(x4)
  sd x5, 1264(x4)
  sd x5, 1272(x4)
  sd x5, 1280(x4)
  sd x5, 1288(x4)
  sd x5, 1296(x4)
  sd x5, 1304(x4)
  sd x5, 1312(x4)
  sd x5, 1320(x4)
  sd x5, 1328(x4)
  sd x5, 1336(x4)
  sd x5, 1344(x4)
  sd x5, 1352(x4)
  sd x5, 1360(x4)
  sd x5, 1368(x4)
  sd x5, 1376(x4)
  sd x5, 1384(x4)
  sd x5, 1392(x4)
  sd x5, 1400(x4)
  sd x5, 1408(x4)
  sd x5, 1416(x4)
  sd x5, 1424(x4)
  sd x5, 1432(x4)
  sd x5, 1440(x4)
  sd x5, 1448(x4)
  sd x5, 1456(x4)
  sd x5, 1464(x4)
  sd x5, 1472(x4)
  sd x5, 1480(x4)
  sd x5, 1488(x4)
  sd x5, 1496(x4)
  sd x5, 1504(x4)
  sd x5, 1512(x4)
  sd x5, 1520(x4)
  sd x5, 1528(x4)
  sd x5, 1536(x4)
  sd x5, 1544(x4)
  sd x5, 1552(x4)
  sd x5, 1560(x4)
  sd x5, 1568(x4)
  sd x5, 1576(x4)
  sd x5, 1584(x4)
  sd x5, 1592(x4)
  sd x5, 1600(x4)
  sd x5, 1608(x4)
  sd x5, 1616(x4)
  sd x5, 1624(x4)
  sd x5, 1632(x4)
  sd x5, 1640(x4)
  sd x5, 1648(x4)
  sd x5, 1656(x4)
  sd x5, 1664(x4)
  sd x5, 1672(x4)
  sd x5, 1680(x4)
  sd x5, 1688(x4)
  sd x5, 1696(x4)
  sd x5, 1704(x4)
  sd x5, 1712(x4)
  sd x5, 1720(x4)
  sd x5, 1728(x4)
  sd x5, 1736(x4)
  sd x5, 1744(x4)
  sd x5, 1752(x4)
  sd x5, 1760(x4)
  sd x5, 1768(x4)
  sd x5, 1776(x4)
  sd x5, 1784(x4)
  sd x5, 1792(x4)
  sd x5, 1800(x4)
  sd x5, 1808(x4)
  sd x5, 1816(x4)
  sd x5, 1824(x4)
  sd x5, 1832(x4)
  sd x5, 1840(x4)
  sd x5, 1848(x4)
  sd x5, 1856(x4)
  sd x5, 1864(x4)
  sd x5, 1872(x4)
  sd x5, 1880(x4)
  sd x5, 1888(x4)
  sd x5, 1896(x4)
  sd x5, 1904(x4)
  sd x5, 1912(x4)
  sd x5, 1920(x4)
  sd x5, 1928(x4)
  sd x5, 1936(x4)
  sd x5, 1944(x4)
  sd x5, 1952(x4)
  sd x5, 1960(x4)
  sd x5, 1968(x4)
  sd x5, 1976(x4)
  sd x5, 1984(x4)
  sd x5, 1992(x4)
  sd x5, 2000(x4)
  sd x5, 2008(x4)
  sd x5, 2016(x4)
  sd x5, 2024(x4)
  sd x5, 2032(x4)
  sd x5, 2040(x4)

  li x5, 0x546be46ca457238e

  andi x5, x18, 1400
  lb x17, 400(x4)
  sltiu x14, x14, 496
  xor x10, x9, x7
  sll x13, x29, x28
  sd x25, 624(x4)
  sw x26, 624(x4)
  ld x18, 624(x4)
  sll x31, x22, x17
  sltiu x28, x12, 769
  addi x12, x7, 1123
  mulhu x6, x20, x7
  bltu x12, x27, .Ltaken_10003_0
  mulhsu x8, x26, x16
  j .Lend_10003_0
.Ltaken_10003_0:
  sltiu x12, x28, 1642
.Lend_10003_0:
  bltu x13, x13, .Ltaken_10003_1
  mulhsu x30, x15, x29
  sraiw x26, x19, 28
  j .Lend_10003_1
.Ltaken_10003_1:
  mulh x18, x20, x15
  mul x5, x15, x17
  add x31, x12, x19
.Lend_10003_1:
  bltu x31, x25, .Ltaken_10003_2
  or  x12, x29, x8
  subw x17, x6, x25
  slti x27, x18, -1315
  j .Lend_10003_2
.Ltaken_10003_2:
  mulhu x28, x6, x15
  sltu x31, x10, x18
  sltu x16, x18, x14
.Lend_10003_2:
  lb x27, 320(x4)
  sra x26, x27, x19
  sw x23, 1240(x4)
  ori  x7, x22, 1739
  mulhu x28, x17, x31
  # Speculative memory test
  sd x31, 544(x4)
  bge x31, x29, .Lspec_taken_10003_0
  sd x29, 544(x4)
  ld x21, 888(x4)
  addw x18, x28, x31
  j .Lspec_end_10003_0
.Lspec_taken_10003_0:
  sd x31, 888(x4)
  ld x21, 544(x4)
  andi x14, x8, 903
.Lspec_end_10003_0:
  ld x21, 544(x4)
  addi x29, x25, 963
  add x29, x25, x8
  addi x19, x15, -901
  add x16, x19, x15
  xor x15, x16, x19
  sb x17, 1576(x4)
  sltu x28, x24, x25
  subw x10, x14, x28
  ld x11, 1576(x4)
  or  x31, x15, x11
  lbu x25, 960(x4)
  addi x11, x6, 1300
  add x28, x11, x6
  xor x6, x28, x11
  sw x22, 1352(x4)
  sw x31, 496(x4)
  mul x25, x9, x28
  mulw x6, x12, x12
  bge x23, x31, .Ltaken_10003_3
  xori x24, x6, -1452
  mulhsu x17, x17, x30
  sub x23, x8, x23
  mulhu x24, x11, x20
  j .Lend_10003_3
.Ltaken_10003_3:
  subw x26, x23, x11
  addiw x16, x18, -166
  srai x28, x16, 58
.Lend_10003_3:
  lh x23, 264(x4)
  sb x30, 1192(x4)
  addi x16, x4, 1552
  lr.d x24, (x16)
  sc.d x29, x22, (x16)
  addi x19, x4, 56
  sd x9, 0(x19)
  amoand.d x31, x9, (x19)
  ld x22, 0(x19)
  blt x31, x26, .Ltaken_10003_4
  slt x20, x8, x7
  sraiw x19, x28, 8
  subw x15, x10, x9
  j .Lend_10003_4
.Ltaken_10003_4:
  sraw x8, x29, x13
.Lend_10003_4:
  xor x6, x23, x30
  lbu x27, 1536(x4)
  sd x7, 1880(x4)
  srlw x9, x31, x18
  fence
  ld x9, 1880(x4)
  sh x21, 1168(x4)
  div x20, x5, x17
  sb x6, 1512(x4)
  slti x8, x7, 1129
  sd x24, 1272(x4)
  fence rw, rw
  ld x12, 1272(x4)
  sltiu x16, x10, 375
  ld x24, 768(x4)
  sllw x21, x20, x9
  andi x16, x6, 990
  slliw x25, x11, 14
  andi x10, x16, 1001
  sd x28, 1248(x4)
  sw x22, 1248(x4)
  ld x26, 1248(x4)
  addi x27, x4, 1832
  sd x5, 0(x27)
  amoor.w x26, x5, (x27)
  ld x14, 0(x27)
  lwu x13, 1880(x4)
  bgeu x6, x10, .Ltaken_10003_5
  andi x12, x6, -758
  srl x6, x25, x25
  j .Lend_10003_5
.Ltaken_10003_5:
  srai x22, x7, 14
  mulhu x17, x30, x24
.Lend_10003_5:
  sllw x31, x8, x17
  slt x28, x19, x13
  addi x19, x4, 840
  sd x17, 0(x19)
  amoand.d x17, x17, (x19)
  ld x12, 0(x19)
  lhu x9, 1272(x4)
  ori  x10, x21, -1402
  sraw x7, x15, x22
  sub x13, x27, x22
  sd x20, 1712(x4)
  slliw x17, x13, 18
  fence.i
  ld x18, 1712(x4)
  # Pointer chase pattern
  addi x18, x4, 568
  sd x18, 128(x4)
  addiw x12, x25, -661
  ld x18, 128(x4)
  ld x17, 0(x18)
  sllw x29, x28, x29
  bne x29, x10, .Ltaken_10003_6
  addi x23, x18, 1047
  srliw x22, x30, 3
  slli x28, x30, 47
  addw x17, x24, x22
  j .Lend_10003_6
.Ltaken_10003_6:
  sraiw x5, x15, 1
.Lend_10003_6:
  ori  x8, x8, 121
  lb x16, 968(x4)
  rem x7, x11, x24
  sb x9, 352(x4)
  ld x11, 352(x4)
  rem x12, x27, x11
  # Speculative memory test
  sd x17, 560(x4)
  bne x17, x5, .Lspec_taken_10003_1
  sd x5, 560(x4)
  ld x6, 928(x4)
  srliw x15, x16, 6
  j .Lspec_end_10003_1
.Lspec_taken_10003_1:
  sd x17, 928(x4)
  ld x6, 560(x4)
  ori  x6, x29, -1727
.Lspec_end_10003_1:
  ld x6, 560(x4)
  addi x25, x4, 1744
  lr.w x16, (x25)
  sc.w x19, x27, (x25)
  sd x25, 1240(x4)
  sb x12, 200(x4)
  sw x5, 1728(x4)
  sb x5, 600(x4)
  sb x15, 624(x4)
  sd x20, 1400(x4)
  sd x24, 1576(x4)
  sb x6, 1680(x4)
  sh x27, 1136(x4)
  sw x19, 1920(x4)
  sb x6, 1704(x4)
  lw x16, 600(x4)
  lhu x29, 1920(x4)
  lui x14, 1048521
  sd x12, 944(x4)
  sw x17, 2024(x4)
  # Speculative memory test
  sd x30, 1272(x4)
  beq x30, x28, .Lspec_taken_10003_2
  sd x28, 1272(x4)
  ld x25, 1488(x4)
  slt x29, x10, x10
  j .Lspec_end_10003_2
.Lspec_taken_10003_2:
  sd x30, 1488(x4)
  ld x25, 1272(x4)
  lui x10, 1047753
.Lspec_end_10003_2:
  ld x25, 1272(x4)
  bltu x31, x7, .Ltaken_10003_7
  or  x28, x10, x7
  ori  x18, x29, -1717
  and x24, x21, x11
  j .Lend_10003_7
.Ltaken_10003_7:
  xori x26, x9, -1651
  lui x25, 1429
  mulhu x5, x29, x16
.Lend_10003_7:

  # Dump register state to signature
  la x4, begin_signature
  sd x5, 0(x4)
  sd x6, 8(x4)
  sd x7, 16(x4)
  sd x8, 24(x4)
  sd x9, 32(x4)
  sd x10, 40(x4)
  sd x11, 48(x4)
  sd x12, 56(x4)
  sd x13, 64(x4)
  sd x14, 72(x4)
  sd x15, 80(x4)
  sd x16, 88(x4)
  sd x17, 96(x4)
  sd x18, 104(x4)
  sd x19, 112(x4)
  sd x20, 120(x4)
  sd x21, 128(x4)
  sd x22, 136(x4)
  sd x23, 144(x4)
  sd x24, 152(x4)
  sd x25, 160(x4)
  sd x26, 168(x4)
  sd x27, 176(x4)
  sd x28, 184(x4)
  sd x29, 192(x4)
  sd x30, 200(x4)
  sd x31, 208(x4)

  RVTEST_PASS

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN

  .align 4

  # Scratch memory for loads/stores (2KB, 8-byte aligned)
  .align 4
scratch_data:
  .fill 2048, 1, 0

RVTEST_DATA_END