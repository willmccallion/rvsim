//! Vector arithmetic funct6 values (bits 31:26).
//!
//! Note: The same funct6 value may map to different operations depending on
//! the funct3 category (OPIVV vs OPMVV vs OPFVV, etc.).

// ── Integer arithmetic (OPIVV/OPIVX/OPIVI) ──────────────────────────────────

/// Vector add (`vadd`).
pub const VADD: u32 = 0b000000;
/// Vector subtract (`vsub`).
pub const VSUB: u32 = 0b000010;
/// Vector reverse subtract (`vrsub`).
pub const VRSUB: u32 = 0b000011;
/// Vector unsigned minimum (`vminu`).
pub const VMINU: u32 = 0b000100;
/// Vector signed minimum (`vmin`).
pub const VMIN: u32 = 0b000101;
/// Vector unsigned maximum (`vmaxu`).
pub const VMAXU: u32 = 0b000110;
/// Vector signed maximum (`vmax`).
pub const VMAX: u32 = 0b000111;
/// Vector bitwise AND (`vand`).
pub const VAND: u32 = 0b001001;
/// Vector bitwise OR (`vor`).
pub const VOR: u32 = 0b001010;
/// Vector bitwise XOR (`vxor`).
pub const VXOR: u32 = 0b001011;
/// Vector register gather (`vrgather`).
pub const VRGATHER: u32 = 0b001100;
/// Vector register gather with 16-bit indices (`vrgatherei16`).
pub const VRGATHEREI16: u32 = 0b001110;
/// Vector slide up (`vslideup`).
pub const VSLIDEUP: u32 = 0b001110;
/// Vector slide down (`vslidedown`).
pub const VSLIDEDOWN: u32 = 0b001111;

// ── Add/sub with carry ──────────────────────────────────────────────────────

/// Vector add with carry (`vadc`).
pub const VADC: u32 = 0b010000;
/// Vector mask add with carry (`vmadc`).
pub const VMADC: u32 = 0b010001;
/// Vector subtract with borrow (`vsbc`).
pub const VSBC: u32 = 0b010010;
/// Vector mask subtract with borrow (`vmsbc`).
pub const VMSBC: u32 = 0b010011;

// ── Merge/move ──────────────────────────────────────────────────────────────

/// Vector merge / move (`vmerge` / `vmv`).
pub const VMERGE_VMV: u32 = 0b010111;

// ── Comparison (write mask result) ──────────────────────────────────────────

/// Vector mask set if equal (`vmseq`).
pub const VMSEQ: u32 = 0b011000;
/// Vector mask set if not equal (`vmsne`).
pub const VMSNE: u32 = 0b011001;
/// Vector mask set if less than unsigned (`vmsltu`).
pub const VMSLTU: u32 = 0b011010;
/// Vector mask set if less than signed (`vmslt`).
pub const VMSLT: u32 = 0b011011;
/// Vector mask set if less than or equal unsigned (`vmsleu`).
pub const VMSLEU: u32 = 0b011100;
/// Vector mask set if less than or equal signed (`vmsle`).
pub const VMSLE: u32 = 0b011101;
/// Vector mask set if greater than unsigned (`vmsgtu`).
pub const VMSGTU: u32 = 0b011110;
/// Vector mask set if greater than signed (`vmsgt`).
pub const VMSGT: u32 = 0b011111;

// ── Fixed-point (OPIVV/OPIVX/OPIVI) ────────────────────────────────────────

/// Vector saturating add unsigned (`vsaddu`).
pub const VSADDU: u32 = 0b100000;
/// Vector saturating add signed (`vsadd`).
pub const VSADD: u32 = 0b100001;
/// Vector saturating subtract unsigned (`vssubu`).
pub const VSSUBU: u32 = 0b100010;
/// Vector saturating subtract signed (`vssub`).
pub const VSSUB: u32 = 0b100011;
/// Vector shift left logical (`vsll`).
pub const VSLL: u32 = 0b100101;
/// Vector signed fractional multiply (`vsmul`).
pub const VSMUL: u32 = 0b100111;
/// Vector shift right logical (`vsrl`).
pub const VSRL: u32 = 0b101000;
/// Vector shift right arithmetic (`vsra`).
pub const VSRA: u32 = 0b101001;
/// Vector scaling shift right logical (`vssrl`).
pub const VSSRL: u32 = 0b101010;
/// Vector scaling shift right arithmetic (`vssra`).
pub const VSSRA: u32 = 0b101011;

// ── Narrowing ───────────────────────────────────────────────────────────────

/// Vector narrowing shift right logical (`vnsrl`).
pub const VNSRL: u32 = 0b101100;
/// Vector narrowing shift right arithmetic (`vnsra`).
pub const VNSRA: u32 = 0b101101;
/// Vector narrowing clip unsigned (`vnclipu`).
pub const VNCLIPU: u32 = 0b101110;
/// Vector narrowing clip signed (`vnclip`).
pub const VNCLIP: u32 = 0b101111;

// ── Widening reduction (OPMVV) ─────────────────────────────────────────────

/// Vector widening unsigned reduction sum (`vwredsumu`).
pub const VWREDSUMU: u32 = 0b110000;
/// Vector widening signed reduction sum (`vwredsum`).
pub const VWREDSUM: u32 = 0b110001;

// ── Widening integer add/sub (OPMVV/OPMVX) ─────────────────────────────────

/// Vector widening unsigned add (`vwaddu`).
pub const VWADDU: u32 = 0b110000;
/// Vector widening signed add (`vwadd`).
pub const VWADD: u32 = 0b110001;
/// Vector widening unsigned subtract (`vwsubu`).
pub const VWSUBU: u32 = 0b110010;
/// Vector widening signed subtract (`vwsub`).
pub const VWSUB: u32 = 0b110011;
/// Vector widening unsigned add wide (`vwaddu.w`).
pub const VWADDU_W: u32 = 0b110100;
/// Vector widening signed add wide (`vwadd.w`).
pub const VWADD_W: u32 = 0b110101;
/// Vector widening unsigned subtract wide (`vwsubu.w`).
pub const VWSUBU_W: u32 = 0b110110;
/// Vector widening signed subtract wide (`vwsub.w`).
pub const VWSUB_W: u32 = 0b110111;

// ── Widening multiply (OPMVV/OPMVX) ────────────────────────────────────────

/// Vector widening unsigned multiply (`vwmulu`).
pub const VWMULU: u32 = 0b111000;
/// Vector widening signed-unsigned multiply (`vwmulsu`).
pub const VWMULSU: u32 = 0b111010;
/// Vector widening signed multiply (`vwmul`).
pub const VWMUL: u32 = 0b111011;
/// Vector widening unsigned multiply-accumulate (`vwmaccu`).
pub const VWMACCU: u32 = 0b111100;
/// Vector widening signed multiply-accumulate (`vwmacc`).
pub const VWMACC: u32 = 0b111101;
/// Vector widening unsigned-signed multiply-accumulate (`vwmaccus`).
pub const VWMACCUS: u32 = 0b111110;
/// Vector widening signed-unsigned multiply-accumulate (`vwmaccsu`).
pub const VWMACCSU: u32 = 0b111111;

// ── Integer multiply/divide (OPMVV/OPMVX) ──────────────────────────────────

/// Vector unsigned divide (`vdivu`).
pub const VDIVU: u32 = 0b100000;
/// Vector signed divide (`vdiv`).
pub const VDIV: u32 = 0b100001;
/// Vector unsigned remainder (`vremu`).
pub const VREMU: u32 = 0b100010;
/// Vector signed remainder (`vrem`).
pub const VREM: u32 = 0b100011;
/// Vector multiply high unsigned (`vmulhu`).
pub const VMULHU: u32 = 0b100100;
/// Vector multiply low bits (`vmul`).
pub const VMUL: u32 = 0b100101;
/// Vector multiply high signed-unsigned (`vmulhsu`).
pub const VMULHSU: u32 = 0b100110;
/// Vector multiply high signed (`vmulh`).
pub const VMULH: u32 = 0b100111;
/// Vector multiply-add overwriting addend (`vmadd`).
pub const VMADD: u32 = 0b101001;
/// Vector negated multiply-subtract overwriting addend (`vnmsub`).
pub const VNMSUB: u32 = 0b101011;
/// Vector multiply-accumulate overwriting addend (`vmacc`).
pub const VMACC: u32 = 0b101101;
/// Vector negated multiply-subtract accumulate (`vnmsac`).
pub const VNMSAC: u32 = 0b101111;

// ── Integer reduction (OPMVV) ───────────────────────────────────────────────

/// Vector reduction sum (`vredsum`).
pub const VREDSUM: u32 = 0b000000;
/// Vector reduction AND (`vredand`).
pub const VREDAND: u32 = 0b000001;
/// Vector reduction OR (`vredor`).
pub const VREDOR: u32 = 0b000010;
/// Vector reduction XOR (`vredxor`).
pub const VREDXOR: u32 = 0b000011;
/// Vector reduction unsigned minimum (`vredminu`).
pub const VREDMINU: u32 = 0b000100;
/// Vector reduction signed minimum (`vredmin`).
pub const VREDMIN: u32 = 0b000101;
/// Vector reduction unsigned maximum (`vredmaxu`).
pub const VREDMAXU: u32 = 0b000110;
/// Vector reduction signed maximum (`vredmax`).
pub const VREDMAX: u32 = 0b000111;

// ── Averaging add/sub (OPMVV/OPMVX) ────────────────────────────────────────

/// Vector averaging unsigned add (`vaaddu`).
pub const VAADDU: u32 = 0b001000;
/// Vector averaging signed add (`vaadd`).
pub const VAADD: u32 = 0b001001;
/// Vector averaging unsigned subtract (`vasubu`).
pub const VASUBU: u32 = 0b001010;
/// Vector averaging signed subtract (`vasub`).
pub const VASUB: u32 = 0b001011;

// ── FP arithmetic (OPFVV/OPFVF) ────────────────────────────────────────────

/// Vector FP add (`vfadd`).
pub const VFADD: u32 = 0b000000;
/// Vector FP unordered reduction sum (`vfredusum`).
pub const VFREDUSUM: u32 = 0b000001;
/// Vector FP subtract (`vfsub`).
pub const VFSUB: u32 = 0b000010;
/// Vector FP ordered reduction sum (`vfredosum`).
pub const VFREDOSUM: u32 = 0b000011;
/// Vector FP minimum (`vfmin`).
pub const VFMIN: u32 = 0b000100;
/// Vector FP reduction minimum (`vfredmin`).
pub const VFREDMIN: u32 = 0b000101;
/// Vector FP maximum (`vfmax`).
pub const VFMAX: u32 = 0b000110;
/// Vector FP reduction maximum (`vfredmax`).
pub const VFREDMAX: u32 = 0b000111;
/// Vector FP sign injection (`vfsgnj`).
pub const VFSGNJ: u32 = 0b001000;
/// Vector FP negated sign injection (`vfsgnjn`).
pub const VFSGNJN: u32 = 0b001001;
/// Vector FP XOR sign injection (`vfsgnjx`).
pub const VFSGNJX: u32 = 0b001010;
/// Vector FP slide one up (`vfslide1up`).
pub const VFSLIDE1UP: u32 = 0b001110;
/// Vector FP slide one down (`vfslide1down`).
pub const VFSLIDE1DOWN: u32 = 0b001111;

// ── FP conversion/class/sqrt (OPFVV, vs1 field encodes variant) ─────────────

/// Vector FP widening-unary0 encoding (`vfmv.f.s`; OPFVV funct6=010000).
pub const VWFUNARY0: u32 = 0b010000;

// ── vs1 sub-field values for unary operations ───────────────────────────────

/// vs1 sub-field for `vfmv.f.s` within VWFUNARY0.
pub const VWFUNARY0_VFMV_F_S: u8 = 0b00000;
/// Vector FP reverse-unary0 encoding (`vfmv.s.f`; OPFVF funct6=010000).
/// Same numeric value as VWFUNARY0 but used in OPFVF context.
pub const VRFUNARY0: u32 = 0b010000;
/// vs2 sub-field for `vfmv.s.f` within VRFUNARY0.
pub const VRFUNARY0_VFMV_S_F: u8 = 0b00000;
/// Vector FP unary0 encoding (`vfcvt`, `vfwcvt`, `vfncvt`).
pub const VFUNARY0: u32 = 0b010010;
/// Vector FP unary1 encoding (`vfsqrt`, `vfclass`, `vfrec7`, `vfrsqrt7`).
pub const VFUNARY1: u32 = 0b010011;

// ── FP comparison (OPFVV/OPFVF) ────────────────────────────────────────────

/// Vector FP mask set if equal (`vmfeq`).
pub const VMFEQ: u32 = 0b011000;
/// Vector FP mask set if less than or equal (`vmfle`).
pub const VMFLE: u32 = 0b011001;
/// Vector FP mask set if ordered (`vmford`).
pub const VMFORD: u32 = 0b011010;
/// Vector FP mask set if less than (`vmflt`).
pub const VMFLT: u32 = 0b011011;
/// Vector FP mask set if not equal (`vmfne`).
pub const VMFNE: u32 = 0b011100;
/// Vector FP mask set if greater than (`vmfgt`).
pub const VMFGT: u32 = 0b011101;
/// Vector FP mask set if greater than or equal (`vmfge`).
pub const VMFGE: u32 = 0b011110;

// ── FP multiply/divide (OPFVV/OPFVF) ───────────────────────────────────────

/// Vector FP divide (`vfdiv`).
pub const VFDIV: u32 = 0b100000;
/// Vector FP reverse divide (`vfrdiv`).
pub const VFRDIV: u32 = 0b100001;
/// Vector FP multiply (`vfmul`).
pub const VFMUL: u32 = 0b100100;

// ── FP fused multiply-add (OPFVV/OPFVF) ────────────────────────────────────

/// Vector FP multiply-add (`vfmadd`).
pub const VFMADD: u32 = 0b101000;
/// Vector FP negated multiply-add (`vfnmadd`).
pub const VFNMADD: u32 = 0b101001;
/// Vector FP multiply-subtract (`vfmsub`).
pub const VFMSUB: u32 = 0b101010;
/// Vector FP negated multiply-subtract (`vfnmsub`).
pub const VFNMSUB: u32 = 0b101011;
/// Vector FP multiply-add accumulate (`vfmacc`).
pub const VFMACC: u32 = 0b101100;
/// Vector FP negated multiply-add accumulate (`vfnmacc`).
pub const VFNMACC: u32 = 0b101101;
/// Vector FP multiply-subtract accumulate (`vfmsac`).
pub const VFMSAC: u32 = 0b101110;
/// Vector FP negated multiply-subtract accumulate (`vfnmsac`).
pub const VFNMSAC: u32 = 0b101111;

// ── FP widening (OPFVV/OPFVF) ──────────────────────────────────────────────

/// Vector FP widening add (`vfwadd`).
pub const VFWADD: u32 = 0b110000;
/// Vector FP widening unordered reduction sum (`vfwredusum`).
pub const VFWREDUSUM: u32 = 0b110001;
/// Vector FP widening subtract (`vfwsub`).
pub const VFWSUB: u32 = 0b110010;
/// Vector FP widening ordered reduction sum (`vfwredosum`).
pub const VFWREDOSUM: u32 = 0b110011;
/// Vector FP widening add wide (`vfwadd.w`).
pub const VFWADD_W: u32 = 0b110100;
/// Vector FP widening subtract wide (`vfwsub.w`).
pub const VFWSUB_W: u32 = 0b110110;
/// Vector FP widening multiply (`vfwmul`).
pub const VFWMUL: u32 = 0b111000;
/// Vector FP widening multiply-add accumulate (`vfwmacc`).
pub const VFWMACC: u32 = 0b111100;
/// Vector FP widening negated multiply-add accumulate (`vfwnmacc`).
pub const VFWNMACC: u32 = 0b111101;
/// Vector FP widening multiply-subtract accumulate (`vfwmsac`).
pub const VFWMSAC: u32 = 0b111110;
/// Vector FP widening negated multiply-subtract accumulate (`vfwnmsac`).
pub const VFWNMSAC: u32 = 0b111111;

// ── OPMVV unary groups ────────────────────────────────────────────────────

/// Unary scalar-result ops in OPMVV: `vmv.x.s`, `vcpop.m`, `vfirst.m`.
pub const VWXUNARY0: u32 = 0b010000;
/// Integer extension unary ops in OPMVV: `vzext`, `vsext`.
pub const VXUNARY0: u32 = 0b010010;
/// Mask-source unary ops in OPMVV: `vmsbf.m`, `vmsof.m`, `vmsif.m`, `viota.m`, `vid.v`.
pub const VMUNARY0: u32 = 0b010100;

// ── vs1 sub-field values for OPMVV unary operations ─────────────────────

/// vs1 for `vmv.x.s` within VWXUNARY0.
pub const VWXUNARY0_VMV_X_S: u8 = 0b00000;
/// vs1 for `vcpop.m` within VWXUNARY0.
pub const VWXUNARY0_VCPOP_M: u8 = 0b10000;
/// vs1 for `vfirst.m` within VWXUNARY0.
pub const VWXUNARY0_VFIRST_M: u8 = 0b10001;
/// vs1 for `vzext.vf8` within VXUNARY0.
pub const VXUNARY0_VZEXT_VF8: u8 = 0b00010;
/// vs1 for `vsext.vf8` within VXUNARY0.
pub const VXUNARY0_VSEXT_VF8: u8 = 0b00011;
/// vs1 for `vzext.vf4` within VXUNARY0.
pub const VXUNARY0_VZEXT_VF4: u8 = 0b00100;
/// vs1 for `vsext.vf4` within VXUNARY0.
pub const VXUNARY0_VSEXT_VF4: u8 = 0b00101;
/// vs1 for `vzext.vf2` within VXUNARY0.
pub const VXUNARY0_VZEXT_VF2: u8 = 0b00110;
/// vs1 for `vsext.vf2` within VXUNARY0.
pub const VXUNARY0_VSEXT_VF2: u8 = 0b00111;
/// vs1 for `vmsbf.m` within VMUNARY0.
pub const VMUNARY0_VMSBF_M: u8 = 0b00001;
/// vs1 for `vmsof.m` within VMUNARY0.
pub const VMUNARY0_VMSOF_M: u8 = 0b00010;
/// vs1 for `vmsif.m` within VMUNARY0.
pub const VMUNARY0_VMSIF_M: u8 = 0b00011;
/// vs1 for `viota.m` within VMUNARY0.
pub const VMUNARY0_VIOTA_M: u8 = 0b10000;
/// vs1 for `vid.v` within VMUNARY0.
pub const VMUNARY0_VID_V: u8 = 0b10001;

// ── Mask operations (OPMVV) ─────────────────────────────────────────────────

/// Mask AND (`vmand.mm`).
pub const VMAND: u32 = 0b011001;
/// Mask NAND (`vmnand.mm`).
pub const VMNAND: u32 = 0b011101;
/// Mask AND-NOT (`vmandn.mm`).
pub const VMANDN: u32 = 0b011000;
/// Mask XOR (`vmxor.mm`).
pub const VMXOR: u32 = 0b011011;
/// Mask OR (`vmor.mm`).
pub const VMOR: u32 = 0b011010;
/// Mask NOR (`vmnor.mm`).
pub const VMNOR: u32 = 0b011110;
/// Mask OR-NOT (`vmorn.mm`).
pub const VMORN: u32 = 0b011100;
/// Mask XNOR (`vmxnor.mm`).
pub const VMXNOR: u32 = 0b011111;
