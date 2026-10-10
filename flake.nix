{
  description = "rvsim — RISC-V cycle-level simulator dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Chipyard 1.14.0 needs CIRCT's firtool 1.75.0 (conda-reqs/circt.json):
    # later firtools reject rocket-chip's printf-encoded verification ops.
    # nixos-unstable of 2024-06-06 is the last channel that carries it, and
    # its build is cached.
    nixpkgs-circt.url = "github:NixOS/nixpkgs/dd443d2d10ff7f7f7451f5bd04268f21daceb74d";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, nixpkgs-circt, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        firtool = (import nixpkgs-circt { inherit system; }).circt;
        # The same rust-toolchain.toml that rustup (and CI) reads.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        # Cross toolchain is referenced via store path, not put in PATH —
        # otherwise spike's autoconf picks `riscv64-none-elf-gcc` for the
        # native build and fails with "cannot run C compiled programs".
        riscvGcc = pkgs.pkgsCross.riscv64-embedded.buildPackages.gcc;
        riscvBinutils = pkgs.pkgsCross.riscv64-embedded.buildPackages.binutils;

        # Buildroot insists on /usr/bin/file (libtool legacy); on NixOS
        # there is no /usr/bin, so building Linux requires a FHS-shaped
        # filesystem.  buildFHSEnv composes one via bubblewrap.  The
        # `linux` Makefile target re-execs through this shell when not
        # already inside it (sentinel: RVSIM_FHS_ACTIVE=1).
        fhsEnv = pkgs.buildFHSEnv {
          name = "rvsim-fhs";
          targetPkgs = pkgs: with pkgs; [
            bash coreutils findutils gnused gawk gnugrep gnutar gzip bzip2 xz
            which patch diffutils
            gnumake gcc binutils pkg-config
            file bc cpio unzip rsync perl wget ncurses
            flex bison elfutils openssl
            python3 git curl
          ];
          runScript = "bash";
          profile = ''
            export RVSIM_FHS_ACTIVE=1
            # Disable Nix's gcc-wrapper hardening flags
            # (-Werror=format-security, -D_FORTIFY_SOURCE=2, etc.). Buildroot
            # bootstraps its own host-gcc-initial whose libcpp/libiberty
            # sources don't compile under format-security; we leave hardening
            # to whichever toolchain Buildroot ultimately produces.
            export NIX_HARDENING_ENABLE=
            export NIX_ENFORCE_PURITY=
            # Activate the project's .venv if it exists so the rvsim Python
            # bindings (installed via maturin develop) are importable from
            # inside the FHS env. The venv's python is a /nix/store symlink
            # which bwrap exposes natively.
            if [ -f "$PWD/.venv/bin/activate" ]; then
              # shellcheck disable=SC1091
              . "$PWD/.venv/bin/activate"
            fi
          '';
        };

        devPackages = with pkgs; [
          rustToolchain

          python3
          python3Packages.pip
          python3Packages.virtualenv

          gcc gnumake autoconf automake pkg-config dtc cmake

          go

          git curl
        ];

        # Chipyard's Verilator flow for the RTL comparison (tools/rtl): a
        # JDK for its bundled sbt launcher, the pinned firtool, Verilator,
        # perl and zlib for Verilator's scripts and FST tracing, and jq for
        # its annotation merging. The fesvr library its harness links comes
        # from the spike install.
        rtlPackages = [ firtool ] ++ (with pkgs; [
          openjdk17 verilator perl zlib jq which
        ]);

        devShellHook = ''
          # Defensive: nix stdenv may set CC to a wrapper that resolves to
          # the wrong toolchain. Force native for spike's configure.
          unset CC CXX AR RANLIB

          # Dev-shell builds use every instruction this machine has. CI
          # (which sets CI) and the release wheels build without it, so
          # they test and ship what runs on any x86-64.
          if [ -z "''${CI:-}" ]; then
            export RUSTFLAGS="-C target-cpu=native"
          fi

          export TOOLCHAIN_BIN="$PWD/.nix-toolchain-bin"
          mkdir -p "$TOOLCHAIN_BIN"
          for tool in gcc g++ as ld objdump objcopy strip ar nm ranlib readelf; do
            if [ -e ${riscvGcc}/bin/riscv64-none-elf-$tool ]; then
              ln -sf ${riscvGcc}/bin/riscv64-none-elf-$tool "$TOOLCHAIN_BIN/riscv64-elf-$tool"
            elif [ -e ${riscvBinutils}/bin/riscv64-none-elf-$tool ]; then
              ln -sf ${riscvBinutils}/bin/riscv64-none-elf-$tool "$TOOLCHAIN_BIN/riscv64-elf-$tool"
            fi
          done
          export PATH="$TOOLCHAIN_BIN:$PATH"

          echo "rvsim devshell ready"
          echo "  rust:      $(rustc --version 2>/dev/null)"
          echo "  go:        $(go version 2>/dev/null)"
          echo "  riscv-gcc: $(command -v riscv64-elf-gcc)"
          echo "  native cc: $(command -v gcc)"
          echo "  dtc:       $(command -v dtc)"
        '';
      in {
        packages = {
          fhs = fhsEnv;
        };

        devShells.default = pkgs.mkShell {
          name = "rvsim";
          packages = devPackages;
          shellHook = devShellHook;
        };

        # `nix develop .#rtl`: the default shell plus what builds and runs
        # the Rocket and BOOM Verilator simulators (`make rtl-build`).
        devShells.rtl = pkgs.mkShell {
          name = "rvsim-rtl";
          packages = devPackages ++ rtlPackages;
          shellHook = devShellHook + ''
            export RISCV="$PWD/tests/builds/spike-install"
            echo "  java:      $(java -version 2>&1 | head -1)"
            echo "  firtool:   $(firtool --version 2>/dev/null | head -1)"
            echo "  verilator: $(verilator --version 2>/dev/null)"
            echo "  RISCV:     $RISCV"
          '';
        };
      });
}
