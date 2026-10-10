# What the cached conformance builds are made from: the upstream commits
# spike, riscv-tests and the vector generator are cloned at, and the RVV
# smoke sample. CI caches each build on its own pin (and the vector builds
# on this whole file), so changing one rebuilds only that; change
# CACHE_EPOCH to rebuild them all after editing a build recipe in the
# Makefile.
CACHE_EPOCH := 1

SPIKE_REV        := 20feb9c2bf2a7deab964d8190b0cbd4b4131bec3
RISCV_TESTS_REV  := 1eb47d946c55f55cab8653c224c2993acc0276bd
VECTOR_TESTS_REV := b30515ed611177fd7688fc8129d877698237481a
export VECTOR_TESTS_REV

# Chipyard 1.14.0, whose Rocket and BOOM the RTL comparison (tools/rtl)
# builds as Verilator simulators; not built in CI.
CHIPYARD_REV     := 0acc1e1de2d3284bcd4d876956932a013ffe1949

# A sample of every vector instruction class: integer, fixed-point, widening
# and narrowing, FP and conversions, reductions, masks, permutes, every load
# and store addressing mode, segments, whole registers and the crypto
# extensions.
VECTOR_SMOKE_BUILD   := $(TEST_BUILDS)/vector-smoke
VECTOR_SMOKE_PATTERN := ^(vadd\.(vv|vx|vi)|vsub\.vv|vmul\.vv|vdivu\.vv|vsll\.vi|vsra\.vv|vmin\.vv|vmseq\.vv|vmerge\.vvm|vsadd\.vv|vwadd\.vv|vnsrl\.wv|vzext\.vf2|vfadd\.vv|vfmul\.vf|vfmacc\.vv|vfdiv\.vv|vfsqrt\.v|vfcvt\.x\.f\.v|vfwcvt\.f\.f\.v|vfncvt\.f\.f\.w|vfwmacc\.vv|vfmin\.vv|vfmv\.f\.s|vmv\.x\.s|vredsum\.vs|vfredosum\.vs|vwredsum\.vs|vmand\.mm|vcpop\.m|vfirst\.m|viota\.m|vid\.v|vslideup\.vi|vslidedown\.vx|vrgather\.vv|vcompress\.vm|vmv\.v\.v|vsetvli|vle8\.v|vle32\.v|vle64\.v|vse32\.v|vlse32\.v|vsse64\.v|vluxei32\.v|vsoxei16\.v|vle32ff\.v|vlseg3e16\.v|vsseg2e32\.v|vl2re32\.v|vs4r\.v|vlm\.v|vandn\.vv|vrev8\.v|vclmul\.vv|vghsh\.vv|vaesef\.vv|vsha2ms\.vv|vsm4r\.vv|vsm3me\.vv)$$
