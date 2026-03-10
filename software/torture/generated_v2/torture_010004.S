# Torture test -- seed 10004, length 200
#include "riscv_test.h"
#include "test_macros.h"

RVTEST_RV64U
RVTEST_CODE_BEGIN

  # Initialize registers
  li x5, 0x17fefe5c86e0372d
  li x6, 0x6992fe2ce4600546
  li x7, 0x38855cb91d0ae0a1
  li x8, 0x5a2ca343e3a6a037
  li x9, 0x22aa47cd591a4d55
  li x10, 0x37f9502e2aa6a89c
  li x11, 0x54fc9b8c43af2158
  li x12, 0x158761d9527bdd94
  li x13, 0x750243eef10f2801
  li x14, 0x75f4ad4e5ea092f7
  li x15, 0x51713c8bd81d5d44
  li x16, 0x07233c906e49c372
  li x17, 0x77600df5b9f1ffcf
  li x18, 0x0dbaea5ff5c4001d
  li x19, 0x0fb8c9c0133cd479
  li x20, 0x127c490e2e11d9a5
  li x21, 0x79f1f86cb869d429
  li x22, 0x0efef7927ee0a7de
  li x23, 0x58eb78e8f35c9c60
  li x24, 0x1f465bdd4c1c6eb7
  li x25, 0x6b8f68d1b3e8196e
  li x26, 0x461dacddf5210f2d
  li x27, 0x71bffb711b0d35de
  li x28, 0x40d752d7c51ddf16
  li x29, 0x6e23f7fd68893a5b
  li x30, 0x7a6761165a98beef
  li x31, 0x6b4933ce46df554f

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

  li x5, 0x17fefe5c86e0372d

  sh x31, 352(x4)
  addi x6, x4, 1784
  sd x30, 0(x6)
  amoand.d x27, x30, (x6)
  ld x17, 0(x6)
  addi x8, x23, 903
  add x31, x8, x23
  xor x23, x31, x8
  blt x9, x12, .Ltaken_10004_0
  addi x16, x18, 258
  slti x28, x23, -1282
  j .Lend_10004_0
.Ltaken_10004_0:
  sraiw x13, x6, 14
  sll x25, x19, x7
  mulh x16, x23, x29
.Lend_10004_0:
  sd x10, 1576(x4)
  sltu x20, x7, x22
  fence
  ld x25, 1576(x4)
  sd x29, 1880(x4)
  mulh x13, x16, x5
  ld x26, 1880(x4)
  sltiu x22, x5, 1964
  bltu x5, x19, .Ltaken_10004_1
  sltu x16, x28, x28
  srliw x16, x20, 12
  j .Lend_10004_1
.Ltaken_10004_1:
  srliw x18, x10, 3
  and x16, x7, x21
  andi x24, x30, -113
.Lend_10004_1:
  sd x27, 64(x4)
  mulw x10, x5, x26
  sw x26, 480(x4)
  sd x29, 1872(x4)
  sw x25, 528(x4)
  sh x26, 208(x4)
  sb x21, 728(x4)
  sd x26, 1392(x4)
  sd x9, 1224(x4)
  lh x24, 528(x4)
  ld x19, 1224(x4)
  lhu x15, 1872(x4)
  lhu x15, 1392(x4)
  lb x16, 728(x4)
  ld x13, 208(x4)
  lhu x11, 1224(x4)
  addi x13, x22, -77
  add x13, x22, x16
  sb x6, 1232(x4)
  lb x29, 512(x4)
  srai x30, x13, 43
  sh x10, 1792(x4)
  slt x6, x10, x18
  mulhsu x20, x8, x14
  remw x31, x29, x12
  sraw x29, x22, x10
  ori  x15, x27, -1913
  sw x21, 1008(x4)
  lw x9, 1008(x4)
  addi x22, x4, 1880
  lr.d x25, (x22)
  sc.d x27, x20, (x22)
  srlw x13, x16, x25
  addi x11, x22, -1098
  add x27, x11, x22
  xor x22, x27, x11
  sh x9, 1232(x4)
  sraw x11, x13, x12
  sraiw x30, x26, 29
  mulhsu x25, x9, x15
  # Speculative memory test
  sd x17, 656(x4)
  bge x17, x8, .Lspec_taken_10004_0
  sd x8, 656(x4)
  ld x14, 1936(x4)
  lui x10, 1048224
  j .Lspec_end_10004_0
.Lspec_taken_10004_0:
  sd x17, 1936(x4)
  ld x14, 656(x4)
  addw x21, x22, x20
.Lspec_end_10004_0:
  ld x14, 656(x4)
  slti x11, x25, -482
  sd x21, 1848(x4)
  mulh x10, x28, x9
  ld x24, 1848(x4)
  sd x28, 1152(x4)
  srli x12, x29, 40
  fence w, w
  ld x13, 1152(x4)
  sw x14, 664(x4)
  sw x18, 256(x4)
  sd x31, 1960(x4)
  sh x24, 1112(x4)
  lbu x27, 1112(x4)
  lb x28, 256(x4)
  lbu x28, 664(x4)
  ld x30, 1960(x4)
  addi x15, x26, -51
  add x22, x15, x26
  xor x26, x22, x15
  addi x17, x31, 1973
  add x17, x31, x13
  srlw x16, x9, x25
  srl x15, x9, x27
  sb x8, 200(x4)
  sd x11, 984(x4)
  and x31, x14, x28
  addw x24, x5, x21
  srl x11, x22, x12
  ld x30, 984(x4)
  lb x11, 552(x4)
  bge x19, x10, .Ltaken_10004_2
  mulh x17, x30, x5
  j .Lend_10004_2
.Ltaken_10004_2:
  slliw x24, x31, 30
  srli x27, x19, 5
  addi x19, x13, 879
.Lend_10004_2:
  addi x22, x12, -1565
  add x13, x22, x12
  xor x12, x13, x22
  ld x19, 1768(x4)
  xor x5, x16, x8
  sb x24, 576(x4)
  sw x29, 1120(x4)
  sd x30, 1720(x4)
  fence
  ld x29, 1720(x4)
  lb x27, 1168(x4)
  addi x31, x21, -470
  add x29, x31, x21
  xor x21, x29, x31
  sd x13, 1560(x4)
  slli x26, x7, 32
  mulhsu x17, x29, x24
  ld x6, 1560(x4)
  bge x15, x21, .Ltaken_10004_3
  srli x17, x21, 48
  addw x9, x17, x20
  j .Lend_10004_3
.Ltaken_10004_3:
  sll x22, x13, x17
  slti x5, x27, 1129
.Lend_10004_3:
  addi x8, x4, 600
  sd x7, 0(x8)
  amoxor.w x10, x7, (x8)
  ld x9, 0(x8)
  beq x7, x23, .Ltaken_10004_4
  or  x22, x19, x28
  j .Lend_10004_4
.Ltaken_10004_4:
  addiw x23, x6, -1940
  mulh x15, x6, x25
.Lend_10004_4:
  blt x19, x31, .Ltaken_10004_5
  sltu x27, x5, x14
  sraw x10, x17, x30
  slli x9, x31, 60
  j .Lend_10004_5
.Ltaken_10004_5:
  subw x8, x10, x18
  and x27, x29, x21
  srlw x28, x10, x18
  divw x15, x30, x26
.Lend_10004_5:
  sd x27, 1744(x4)
  sw x18, 1744(x4)
  ld x9, 1744(x4)
  sb x14, 80(x4)
  sb x17, 81(x4)
  lh x25, 80(x4)
  sb x26, 1728(x4)
  lh x29, 648(x4)
  lhu x7, 1680(x4)
  # Speculative memory test
  sd x5, 280(x4)
  beq x5, x26, .Lspec_taken_10004_1
  sd x26, 280(x4)
  ld x8, 1968(x4)
  sub x7, x13, x11
  j .Lspec_end_10004_1
.Lspec_taken_10004_1:
  sd x5, 1968(x4)
  ld x8, 280(x4)
  addiw x9, x13, 1236
.Lspec_end_10004_1:
  ld x8, 280(x4)
  sd x7, 760(x4)
  sltu x25, x20, x8
  ld x13, 760(x4)
  addi x5, x4, 1448
  sd x22, 0(x5)
  amoor.w x24, x22, (x5)
  ld x13, 0(x5)
  bltu x7, x19, .Ltaken_10004_6
  mulhsu x13, x13, x12
  lui x6, 1071
  and x26, x25, x11
  srliw x30, x18, 29
  j .Lend_10004_6
.Ltaken_10004_6:
  slti x27, x16, 105
  or  x19, x21, x31
.Lend_10004_6:
  lwu x6, 856(x4)
  srlw x22, x23, x5
  lbu x13, 944(x4)
  ld x14, 224(x4)
  sra x27, x24, x31
  sb x30, 1808(x4)

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