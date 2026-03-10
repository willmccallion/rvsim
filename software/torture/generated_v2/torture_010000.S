# Torture test -- seed 10000, length 200
#include "riscv_test.h"
#include "test_macros.h"

RVTEST_RV64U
RVTEST_CODE_BEGIN

  # Initialize registers
  li x5, 0x59739a9c01258724
  li x6, 0x32a79582bf67b5cc
  li x7, 0x6e11aeaa2b094a47
  li x8, 0x21bf7f38e5d29720
  li x9, 0x3d5ffe6a8e4339d3
  li x10, 0x312a1085f777eef2
  li x11, 0x51ddb2409a4abd24
  li x12, 0x4b539f7de7047eb0
  li x13, 0x385bbfb8df1013b2
  li x14, 0x4e3352b4de2f325f
  li x15, 0x6532365f64b18245
  li x16, 0x5ddae871177d4059
  li x17, 0x406ae79bf160affe
  li x18, 0x08995d3e4ee7515e
  li x19, 0x0b9cdc32dce9eb1a
  li x20, 0x75f66f5a1af8dd2a
  li x21, 0x6b7ab10e3c0573a9
  li x22, 0x166cbf7d0fae2018
  li x23, 0x44800268d1b764af
  li x24, 0x068a467a441b8ee2
  li x25, 0x64a0d1cc591f0378
  li x26, 0x2ac21aef776f89f7
  li x27, 0x69c3669376ba1e9f
  li x28, 0x0e6f45a6ce6d6663
  li x29, 0x17589a1a169801b7
  li x30, 0x5f8ecfef37933eab
  li x31, 0x52b0bee6a239997f

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

  li x5, 0x59739a9c01258724

  sd x14, 144(x4)
  sh x5, 1064(x4)
  addi x31, x8, 1026
  add x26, x31, x8
  xor x8, x26, x31
  ori  x14, x5, -205
  srliw x21, x10, 11
  sb x6, 1512(x4)
  addw x13, x15, x17
  sh x20, 1032(x4)
  sraw x18, x31, x7
  mulw x9, x19, x24
  lh x22, 1032(x4)
  sd x24, 1064(x4)
  sraw x18, x20, x31
  lw x6, 1068(x4)
  ld x21, 1456(x4)
  sd x7, 944(x4)
  fence rw, rw
  ld x29, 944(x4)
  lwu x10, 1608(x4)
  addi x17, x14, 1448
  add x17, x14, x16
  addi x16, x7, -2011
  add x16, x7, x14
  sll x23, x9, x14
  sh x14, 1424(x4)
  bge x19, x25, .Ltaken_10000_0
  sltu x11, x8, x24
  slt x20, x6, x16
  j .Lend_10000_0
.Ltaken_10000_0:
  mulh x29, x19, x14
  sllw x25, x8, x16
.Lend_10000_0:
  sra x17, x6, x13
  # Pointer chase pattern
  addi x24, x4, 976
  sd x24, 472(x4)
  xor x5, x31, x5
  srlw x21, x30, x5
  ld x24, 472(x4)
  ld x7, 0(x24)
  bge x23, x23, .Ltaken_10000_1
  remw x6, x10, x7
  ori  x17, x24, 723
  srlw x19, x29, x17
  j .Lend_10000_1
.Ltaken_10000_1:
  srlw x24, x19, x8
.Lend_10000_1:
  addi x24, x28, -413
  add x6, x24, x28
  xor x28, x6, x24
  mulhsu x15, x25, x28
  lw x13, 976(x4)
  bltu x16, x7, .Ltaken_10000_2
  add x16, x15, x21
  remw x17, x14, x7
  srai x31, x25, 54
  srlw x16, x7, x8
  j .Lend_10000_2
.Ltaken_10000_2:
  div x11, x13, x10
  sra x19, x31, x26
  srliw x30, x28, 12
.Lend_10000_2:
  bltu x13, x21, .Ltaken_10000_3
  mulhsu x6, x19, x23
  j .Lend_10000_3
.Ltaken_10000_3:
  sltu x24, x14, x25
  addi x26, x10, -368
.Lend_10000_3:
  addi x21, x14, -1642
  add x8, x21, x14
  xor x14, x8, x21
  addi x18, x4, 1864
  sd x15, 0(x18)
  amomin.d x15, x15, (x18)
  ld x5, 0(x18)
  # Pointer chase pattern
  addi x7, x4, 680
  sd x7, 2000(x4)
  ld x7, 2000(x4)
  ld x30, 0(x7)
  bgeu x18, x27, .Ltaken_10000_4
  srlw x7, x29, x17
  j .Lend_10000_4
.Ltaken_10000_4:
  subw x20, x11, x25
  mulhu x20, x13, x28
.Lend_10000_4:
  beq x13, x23, .Ltaken_10000_5
  slliw x27, x11, 3
  j .Lend_10000_5
.Ltaken_10000_5:
  or  x6, x17, x9
  mulhsu x19, x30, x25
  sltu x23, x5, x26
  addi x31, x10, 1010
.Lend_10000_5:
  lb x17, 280(x4)
  subw x16, x28, x24
  lb x6, 1680(x4)
  xor x16, x17, x27
  sd x23, 392(x4)
  fence
  ld x30, 392(x4)
  lh x18, 704(x4)
  xor x8, x20, x16
  bne x19, x17, .Ltaken_10000_6
  srl x22, x29, x26
  mulw x16, x6, x10
  add x15, x24, x17
  j .Lend_10000_6
.Ltaken_10000_6:
  slliw x18, x28, 11
  mulhu x15, x17, x10
  div x28, x10, x14
  srli x15, x22, 10
.Lend_10000_6:
  lb x25, 1064(x4)
  sd x9, 2024(x4)
  sraiw x14, x23, 23
  blt x25, x21, .Ltaken_10000_7
  mulhu x18, x16, x10
  j .Lend_10000_7
.Ltaken_10000_7:
  add x5, x27, x6
.Lend_10000_7:
  bgeu x12, x10, .Ltaken_10000_8
  slti x25, x26, -1958
  srliw x8, x25, 28
  sltiu x25, x7, -1652
  slti x24, x27, 74
  j .Lend_10000_8
.Ltaken_10000_8:
  slt x11, x22, x13
  sraw x8, x11, x23
  slliw x8, x15, 30
  mulh x28, x17, x8
.Lend_10000_8:
  slti x22, x29, -313
  addi x23, x13, 681
  add x17, x23, x13
  xor x13, x17, x23
  sb x17, 1168(x4)
  beq x10, x9, .Ltaken_10000_9
  and x17, x15, x31
  divuw x22, x31, x15
  j .Lend_10000_9
.Ltaken_10000_9:
  srai x31, x27, 37
.Lend_10000_9:
  sd x30, 1072(x4)
  mulh x11, x28, x20
  sltu x9, x23, x29
  lw x19, 1072(x4)
  # Pointer chase pattern
  addi x20, x4, 1752
  sd x20, 312(x4)
  addi x28, x17, 1404
  ld x20, 312(x4)
  ld x20, 0(x20)
  mulhu x11, x17, x28
  sd x25, 2040(x4)
  lbu x24, 632(x4)
  sllw x18, x9, x10
  sd x31, 1728(x4)
  srli x26, x8, 38
  addw x19, x28, x23
  sltiu x27, x23, -902
  ld x27, 1728(x4)
  bge x5, x25, .Ltaken_10000_10
  sltiu x11, x20, 1251
  andi x24, x9, 1109
  sll x20, x11, x5
  srl x28, x9, x14
  j .Lend_10000_10
.Ltaken_10000_10:
  slliw x15, x11, 28
  slliw x26, x17, 25
  addiw x20, x30, -1838
.Lend_10000_10:
  lbu x30, 1784(x4)
  sd x24, 680(x4)
  fence w, w
  ld x17, 680(x4)
  lbu x20, 64(x4)
  lh x20, 960(x4)
  lb x28, 1624(x4)
  sw x10, 776(x4)
  srai x23, x22, 59
  sh x30, 296(x4)
  sd x9, 312(x4)
  sw x23, 312(x4)
  ld x31, 312(x4)
  sd x26, 1584(x4)
  bne x9, x17, .Ltaken_10000_11
  srlw x9, x14, x23
  j .Lend_10000_11
.Ltaken_10000_11:
  sltiu x11, x28, -362
.Lend_10000_11:
  sb x23, 1096(x4)
  sh x19, 24(x4)
  sd x11, 1736(x4)
  fence r, r
  ld x20, 1736(x4)
  srlw x20, x7, x27
  sb x15, 1728(x4)
  sb x7, 1729(x4)
  lh x17, 1728(x4)
  sd x25, 320(x4)
  ld x17, 320(x4)
  lui x17, 1046554
  # Pointer chase pattern
  addi x19, x4, 656
  sd x19, 1256(x4)
  add x26, x28, x9
  ld x19, 1256(x4)
  ld x26, 0(x19)
  sd x8, 2040(x4)

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