// The Chipyard configurations tools/diag/rtl_compare.py runs against. Copied
// into generators/chipyard/src/main/scala/config by tools/rtl/build.sh.
package chipyard

import org.chipsalliance.cde.config.{Config}

// MediumBoomV4Config (2-wide), the BOOM V4 Chipyard's own CI builds,
// printing each retired instruction under +verbose with the cycle it
// retired in (tools/rtl/patches/boom-commit-log-cycle.patch). At this
// Chipyard the 3-wide LargeBoomV4Config hangs on its first CSR write.
class RvsimMediumBoomV4Config extends Config(
  new boom.v4.common.WithBoomCommitLogPrintf ++
  new MediumBoomV4Config)
