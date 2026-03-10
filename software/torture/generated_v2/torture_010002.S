# Torture test -- seed 10002, length 200
#include "riscv_test.h"
#include "test_macros.h"

RVTEST_RV64U
RVTEST_CODE_BEGIN

  # Initialize registers
  li x5, 0x6650051929be710c
  li x6, 0x72fb295194290701
  li x7, 0x6b800a817fd8dcda
  li x8, 0x7e932a5f5bb4d74a
  li x9, 0x62e98cf0ed0422ac
  li x10, 0x5743d3be4c7e27cd
  li x11, 0x169f857e9925c996
  li x12, 0x69d445c7dad25da0
  li x13, 0x7f7ce228e29d82ec
  li x14, 0x1d429007b97b3651
  li x15, 0x57ce9d0f304dee41
  li x16, 0x74bbb4a5f19d19af
  li x17, 0x136fecab874d7dec
  li x18, 0x241532482a651a20
  li x19, 0x6e9a0830a00b6758
  li x20, 0x623d688a61d6f631
  li x21, 0x4f1f31e690b91ce7
  li x22, 0x2b091f2d31e52292
  li x23, 0x0e45e0f9c91c3ee4
  li x24, 0x1f9f01895cb9d317
  li x25, 0x51d281dde9678c0b
  li x26, 0x7366dff1dfe6bf30
  li x27, 0x3780472e0c5e7be6
  li x28, 0x47fda9348ef97af3
  li x29, 0x718eb50a2eaa0d18
  li x30, 0x30fae43f5d3bbc38
  li x31, 0x685ce09fdc3b3e46

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

  li x5, 0x6650051929be710c

  sh x24, 136(x4)
  sd x13, 1000(x4)
  andi x25, x24, 812
  remu x5, x18, x25
  ld x28, 1000(x4)
  sd x21, 912(x4)
  subw x31, x14, x9
  lw x13, 912(x4)
  bne x16, x11, .Ltaken_10002_0
  subw x19, x25, x17
  slliw x30, x27, 24
  ori  x30, x31, -563
  mulhsu x17, x26, x5
  j .Lend_10002_0
.Ltaken_10002_0:
  mulhsu x20, x13, x20
.Lend_10002_0:
  and x21, x6, x11
  sltu x13, x15, x30
  sb x14, 512(x4)
  mul x8, x16, x31
  ld x8, 512(x4)
  sd x5, 1352(x4)
  xori x29, x26, -126
  srli x22, x9, 37
  lh x8, 1354(x4)
  divw x28, x14, x25
  sub x15, x7, x29
  beq x5, x9, .Ltaken_10002_1
  slti x5, x10, -1703
  slli x18, x12, 15
  andi x14, x24, -1680
  j .Lend_10002_1
.Ltaken_10002_1:
  ori  x10, x20, -840
  addw x22, x12, x21
  sraw x19, x19, x7
.Lend_10002_1:
  sh x16, 2024(x4)
  srl x23, x30, x17
  lw x22, 2024(x4)
  and x7, x18, x12
  sw x27, 488(x4)
  lb x13, 888(x4)
  sh x30, 216(x4)
  sd x28, 1672(x4)
  sd x22, 432(x4)
  sb x27, 1720(x4)
  sh x19, 2008(x4)
  sd x28, 888(x4)
  sb x21, 1704(x4)
  sb x19, 1888(x4)
  sh x11, 408(x4)
  sh x20, 1304(x4)
  lbu x11, 432(x4)
  lh x26, 1672(x4)
  lhu x14, 1304(x4)
  lbu x18, 1888(x4)
  lb x29, 408(x4)
  bne x20, x13, .Ltaken_10002_2
  mulhu x30, x5, x15
  j .Lend_10002_2
.Ltaken_10002_2:
  sraw x11, x28, x13
  mul x26, x12, x20
.Lend_10002_2:
  addi x10, x17, 1414
  slt x12, x5, x10
  bne x24, x22, .Ltaken_10002_3
  sraiw x7, x8, 4
  subw x20, x13, x16
  j .Lend_10002_3
.Ltaken_10002_3:
  sra x31, x17, x16
  srliw x30, x7, 0
  slliw x16, x31, 25
.Lend_10002_3:
  srli x29, x7, 22
  xori x21, x30, -220
  lwu x15, 512(x4)
  sd x8, 1576(x4)
  addw x29, x15, x11
  add x26, x18, x15
  lw x24, 1580(x4)
  blt x25, x25, .Ltaken_10002_4
  sraw x27, x25, x20
  addw x21, x5, x29
  sub x31, x10, x27
  j .Lend_10002_4
.Ltaken_10002_4:
  and x9, x9, x13
.Lend_10002_4:
  addw x22, x8, x24
  divu x28, x6, x17
  sw x6, 1632(x4)
  addi x15, x4, 1240
  lr.d x29, (x15)
  sc.d x29, x7, (x15)
  lb x19, 1056(x4)
  sd x20, 1936(x4)
  fence
  ld x27, 1936(x4)
  ld x14, 1904(x4)
  blt x13, x7, .Ltaken_10002_5
  xor x6, x11, x28
  sll x23, x17, x25
  j .Lend_10002_5
.Ltaken_10002_5:
  srliw x19, x15, 31
.Lend_10002_5:
  mulhu x22, x26, x11
  sd x30, 1264(x4)
  sw x19, 1268(x4)
  ld x7, 1264(x4)
  sh x29, 40(x4)
  ld x19, 1784(x4)
  divuw x13, x17, x17
  addi x24, x29, 1343
  add x24, x29, x8
  # Pointer chase pattern
  addi x19, x4, 1664
  sd x19, 80(x4)
  xor x10, x31, x5
  srliw x14, x29, 23
  ld x19, 80(x4)
  ld x16, 0(x19)
  lwu x26, 1976(x4)
  sra x27, x13, x22
  slliw x15, x14, 13
  lw x20, 1560(x4)
  sw x7, 904(x4)
  sub x10, x20, x23
  lw x20, 904(x4)
  sltiu x19, x8, 1653
  sltiu x15, x26, -1108
  ori  x31, x17, -1230
  lwu x9, 1536(x4)
  addi x17, x16, -293
  add x14, x17, x16
  xor x16, x14, x17
  slti x22, x14, -1539
  sh x27, 1224(x4)
  addi x7, x4, 1072
  lr.d x12, (x7)
  sc.d x6, x17, (x7)
  # Speculative memory test
  sd x30, 1944(x4)
  bge x30, x8, .Lspec_taken_10002_0
  sd x8, 1944(x4)
  ld x21, 304(x4)
  sll x10, x17, x17
  j .Lspec_end_10002_0
.Lspec_taken_10002_0:
  sd x30, 304(x4)
  ld x21, 1944(x4)
  mulhu x16, x29, x9
.Lspec_end_10002_0:
  ld x21, 1944(x4)
  lbu x18, 728(x4)
  addw x21, x15, x14
  sw x7, 1528(x4)
  addi x26, x4, 1264
  sd x12, 0(x26)
  amoand.w x21, x12, (x26)
  ld x12, 0(x26)
  sd x9, 152(x4)
  lw x18, 152(x4)
  # Pointer chase pattern
  addi x14, x4, 504
  sd x14, 1568(x4)
  slt x11, x23, x29
  slli x11, x31, 50
  ld x14, 1568(x4)
  ld x7, 0(x14)
  sd x27, 144(x4)
  ld x5, 144(x4)
  sd x19, 496(x4)
  bgeu x30, x22, .Ltaken_10002_6
  srai x27, x25, 13
  mul x9, x23, x23
  j .Lend_10002_6
.Ltaken_10002_6:
  addi x17, x28, 1335
  srlw x25, x14, x12
.Lend_10002_6:
  lh x23, 360(x4)
  sb x18, 1616(x4)
  sb x6, 1808(x4)
  lui x19, 1374
  lhu x27, 552(x4)
  sb x24, 1984(x4)
  sllw x19, x19, x5
  sd x21, 456(x4)
  addi x26, x20, 244
  lh x15, 458(x4)
  xor x5, x6, x30
  blt x9, x11, .Ltaken_10002_7
  srli x24, x5, 2
  mulh x22, x9, x16
  srl x6, x31, x27
  srl x26, x19, x19
  j .Lend_10002_7
.Ltaken_10002_7:
  mulhsu x29, x27, x14
  srlw x17, x14, x24
.Lend_10002_7:
  sw x14, 304(x4)
  sb x17, 1136(x4)
  sb x16, 488(x4)
  sd x30, 40(x4)
  sh x6, 432(x4)
  sw x20, 1104(x4)
  sh x31, 1648(x4)
  lbu x22, 488(x4)
  lhu x14, 1648(x4)
  lh x23, 304(x4)
  lw x13, 40(x4)
  or  x12, x11, x21

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