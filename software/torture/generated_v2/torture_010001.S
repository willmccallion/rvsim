# Torture test -- seed 10001, length 200
#include "riscv_test.h"
#include "test_macros.h"

RVTEST_RV64U
RVTEST_CODE_BEGIN

  # Initialize registers
  li x5, 0x453b33e2399462ce
  li x6, 0x7a9671b87b3d63b2
  li x7, 0x7102c1b28b683bdd
  li x8, 0x093332249a9b903f
  li x9, 0x67ea9802601c5817
  li x10, 0x67e5a2e9fd8d0e0f
  li x11, 0x5df901af29ac08e3
  li x12, 0x12d44c730d9edbdb
  li x13, 0x79994996ac40fad9
  li x14, 0x0f52a5616acc5e54
  li x15, 0x1e342ed99a74f551
  li x16, 0x5ba55ac280473d89
  li x17, 0x13d26e05c142f794
  li x18, 0x543033b50b604b46
  li x19, 0x3f723b77bb0505aa
  li x20, 0x1d8271b631a21387
  li x21, 0x3732fee3a8d2c016
  li x22, 0x4c27453705d240fb
  li x23, 0x01d6ce8336044255
  li x24, 0x0cc6272c8df54333
  li x25, 0x0baffc8226719efb
  li x26, 0x3c9d9d66e1e07065
  li x27, 0x27a468dc6ac2b819
  li x28, 0x0441e1ce39eeba11
  li x29, 0x29d7b37574484133
  li x30, 0x63aea80369cce3f6
  li x31, 0x3fb652d6c716d313

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

  li x5, 0x453b33e2399462ce

  bltu x8, x18, .Ltaken_10001_0
  ori  x25, x7, 150
  slti x5, x20, -629
  and x17, x19, x30
  srai x31, x20, 44
  j .Lend_10001_0
.Ltaken_10001_0:
  lui x8, 536
.Lend_10001_0:
  bne x28, x21, .Ltaken_10001_1
  mul x25, x15, x11
  addiw x29, x16, 2016
  srliw x23, x23, 21
  srli x19, x29, 15
  j .Lend_10001_1
.Ltaken_10001_1:
  addi x8, x16, 342
  slti x23, x9, 671
  srliw x12, x15, 2
.Lend_10001_1:
  lwu x15, 544(x4)
  lwu x15, 2040(x4)
  sd x28, 656(x4)
  sw x8, 660(x4)
  ld x6, 656(x4)
  sb x13, 1720(x4)
  sd x25, 1440(x4)
  sb x7, 304(x4)
  sd x10, 288(x4)
  sd x18, 1504(x4)
  sh x28, 264(x4)
  sw x15, 1736(x4)
  sd x22, 1768(x4)
  sw x30, 488(x4)
  sw x28, 1720(x4)
  lh x18, 1736(x4)
  lbu x8, 1504(x4)
  lwu x25, 288(x4)
  lbu x21, 1720(x4)
  lb x27, 1768(x4)
  bne x20, x8, .Ltaken_10001_2
  addi x30, x19, 373
  j .Lend_10001_2
.Ltaken_10001_2:
  sltiu x27, x25, 481
  slliw x31, x21, 24
  andi x26, x25, -1581
.Lend_10001_2:
  sd x19, 336(x4)
  sh x19, 1976(x4)
  sd x29, 1184(x4)
  sb x29, 400(x4)
  sb x28, 1896(x4)
  sh x8, 640(x4)
  ld x18, 1976(x4)
  lb x23, 1184(x4)
  remuw x24, x30, x28
  lhu x26, 2016(x4)
  slt x5, x25, x25
  sll x17, x24, x12
  addi x14, x29, 238
  add x14, x29, x15
  beq x20, x15, .Ltaken_10001_3
  srl x28, x5, x31
  j .Lend_10001_3
.Ltaken_10001_3:
  slliw x31, x24, 24
.Lend_10001_3:
  sh x21, 1080(x4)
  lb x13, 1192(x4)
  sh x27, 392(x4)
  addi x25, x4, 1912
  lr.w x12, (x25)
  sc.w x16, x29, (x25)
  addi x20, x10, -1013
  add x20, x10, x11
  bne x8, x9, .Ltaken_10001_4
  sraw x27, x24, x13
  or  x24, x24, x19
  addiw x19, x13, 1468
  j .Lend_10001_4
.Ltaken_10001_4:
  mulhu x19, x13, x5
  addi x29, x28, 1080
  lui x10, 1046942
  mulhu x22, x13, x10
.Lend_10001_4:
  sw x25, 840(x4)
  sw x11, 1808(x4)
  sw x25, 920(x4)
  sd x6, 736(x4)
  sw x5, 248(x4)
  sd x28, 1176(x4)
  sb x6, 120(x4)
  sw x18, 1200(x4)
  sw x21, 1504(x4)
  sb x18, 984(x4)
  sd x26, 208(x4)
  lb x19, 920(x4)
  ld x22, 1504(x4)
  ld x19, 208(x4)
  sh x18, 56(x4)
  beq x7, x24, .Ltaken_10001_5
  addi x31, x11, -1128
  sraw x22, x27, x22
  j .Lend_10001_5
.Ltaken_10001_5:
  mulhu x7, x18, x5
  sub x13, x31, x31
.Lend_10001_5:
  bgeu x22, x6, .Ltaken_10001_6
  sub x7, x22, x24
  lui x8, 1047860
  addiw x28, x24, 1301
  j .Lend_10001_6
.Ltaken_10001_6:
  xor x20, x17, x30
  xor x6, x15, x13
  sll x9, x31, x9
.Lend_10001_6:
  lh x30, 0(x4)
  sltu x8, x31, x7
  sh x31, 1344(x4)
  srai x13, x20, 4
  lbu x12, 1400(x4)
  mulhu x21, x6, x5
  # Pointer chase pattern
  addi x16, x4, 1320
  sd x16, 1576(x4)
  mulhu x20, x27, x27
  sll x15, x28, x9
  ld x16, 1576(x4)
  ld x5, 0(x16)
  ld x11, 336(x4)
  sltiu x24, x26, -1593
  mulhsu x22, x6, x26
  sw x10, 1520(x4)
  addi x28, x17, 516
  add x28, x17, x24
  addi x31, x4, 840
  lr.d x17, (x31)
  sc.d x28, x25, (x31)
  bgeu x15, x5, .Ltaken_10001_7
  sltiu x5, x16, -1448
  j .Lend_10001_7
.Ltaken_10001_7:
  sraw x25, x19, x29
.Lend_10001_7:
  divw x31, x25, x22
  sd x25, 1672(x4)
  srliw x18, x24, 24
  sraiw x5, x15, 17
  ld x26, 1672(x4)
  sw x5, 1800(x4)
  addiw x29, x13, 832
  sb x15, 1856(x4)
  sd x27, 504(x4)
  sw x24, 504(x4)
  ld x18, 504(x4)
  or  x24, x18, x17
  bgeu x20, x30, .Ltaken_10001_8
  srli x16, x7, 41
  j .Lend_10001_8
.Ltaken_10001_8:
  sraiw x10, x8, 9
.Lend_10001_8:
  addi x9, x15, 1288
  add x9, x15, x15
  # Speculative memory test
  sd x7, 1064(x4)
  blt x7, x16, .Lspec_taken_10001_0
  sd x16, 1064(x4)
  ld x12, 496(x4)
  xor x17, x25, x14
  j .Lspec_end_10001_0
.Lspec_taken_10001_0:
  sd x7, 496(x4)
  ld x12, 1064(x4)
  or  x26, x30, x15
.Lspec_end_10001_0:
  ld x12, 1064(x4)
  lbu x23, 456(x4)
  sd x9, 1016(x4)
  bge x30, x18, .Ltaken_10001_9
  div x27, x16, x10
  mul x14, x12, x14
  addi x30, x19, 1452
  j .Lend_10001_9
.Ltaken_10001_9:
  slliw x30, x26, 9
.Lend_10001_9:
  # Pointer chase pattern
  addi x27, x4, 584
  sd x27, 88(x4)
  ld x27, 88(x4)
  ld x27, 0(x27)
  lh x30, 1536(x4)
  mulw x20, x19, x9
  bgeu x7, x31, .Ltaken_10001_10
  slti x5, x29, -984
  lui x9, 1047903
  slli x14, x9, 33
  j .Lend_10001_10
.Ltaken_10001_10:
  xor x22, x6, x30
.Lend_10001_10:
  xori x16, x20, -1414
  slt x25, x24, x11
  mulw x10, x30, x9
  lb x26, 1728(x4)
  sd x26, 192(x4)
  mul x10, x12, x23
  fence
  ld x21, 192(x4)
  sw x5, 1160(x4)
  beq x18, x26, .Ltaken_10001_11
  mulw x28, x16, x12
  j .Lend_10001_11
.Ltaken_10001_11:
  srli x5, x14, 59
  subw x27, x8, x7
.Lend_10001_11:
  mulhsu x24, x11, x26
  divu x15, x31, x24
  blt x28, x30, .Ltaken_10001_12
  xor x7, x24, x27
  j .Lend_10001_12
.Ltaken_10001_12:
  sllw x28, x31, x10
.Lend_10001_12:

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