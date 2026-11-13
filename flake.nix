{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {nixpkgs, crane, ...}: let
    # This section recreates env.sh
    cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
    version = cargoToml.workspace.package.version;
    here = ./packaging/linux;
    appID = "io.github.Yoru_KenomoVT.Ayatsuri2D";
    
    forAllSystems = (f: nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed (system:
      let
        # Crane setup
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;
        fileFilter = path: _type:
          pkgs.lib.hasSuffix ".xml" path ||
          pkgs.lib.hasSuffix ".png" path ||
          pkgs.lib.hasSuffix ".tsv" path ||
          pkgs.lib.hasSuffix ".json" path ||
          pkgs.lib.hasSuffix ".h" path ||
          pkgs.lib.hasSuffix "schema.txt" path ||
          pkgs.lib.hasSuffix "CHANGELOG.md" path ||
          pkgs.lib.hasSuffix "LICENSE" path ||
          pkgs.lib.hasSuffix "LICENSING.md" path ||
          pkgs.lib.hasSuffix "README.md" path;
        cargoFilter = path: type: (fileFilter path type) || (craneLib.filterCargoSources path type);
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = cargoFilter;
          name = "source";
        };
        commonArgs = {
          pname = "ayatsuri";
          inherit src;
          strictDeps = true;

          buildInputs = with pkgs; [
            libc
            libgcc
            
            libxkbcommon
            alsa-lib
          ];
          
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        ayatsuri = craneLib.buildPackage (
          commonArgs // {
            inherit cargoArtifacts;
            buildInputs = with pkgs; [
              wayland

              libx11
              libxcursor
              libxi
              
              vulkan-loader
              libGL
              
              ffmpeg
            ] ++ commonArgs.buildInputs;
            cargoBuildExtraArgs = "-p ayatsuri -p ayatsuri-launcher";
            installPhaseCommand = ''
              install -Dm755 "./target/release/ayatsuri" "$out/bin/ayatsuri"
              install -Dm755 "./target/release/ayatsuri-launcher" "$out/bin/ayatsuri-launcher"
              strip "$out/bin/ayatsuri" "$out/bin/ayatsuri-launcher" 2>/dev/null || true
              install -Dm644 "${here}/${appID}.desktop" "$out/share/applications/${appID}.desktop"
              install -Dm644 "${here}/${appID}.mime.xml" "$out/share/mime/packages/${appID}.xml"
              mkdir -p "$out/share/metainfo"
              sed -e "s/@VERSION@/${version}/g" -e "s/@DATE@/$(date -u +%Y-%m-%d)/g" \
                "${here}/${appID}.metainfo.xml.in" > "$out/share/metainfo/${appID}.metainfo.xml"
              mkdir -p "$out/share/icons"
              cp -R "./assets/app-icon/hicolor" "$out/share/icons/"

              mkdir -p "$out/share/doc/ayatsuri2d"
              install -m644 "./LICENSE" "$out/share/doc/ayatsuri2d/LICENSE"
              install -m644 "./LICENSING.md" "$out/share/doc/ayatsuri2d/LICENSING.md"
              install -m644 "./README.md" "$out/share/doc/ayatsuri2d/README.md"
              install -m644 "./assets/credits.tsv" "$out/share/doc/ayatsuri2d/credits.tsv"
            '';
          }
        );

        libayatsuri = craneLib.buildPackage (
          commonArgs // {
            inherit cargoArtifacts;

            nativeBuildInputs = with pkgs; lib.optionals stdenv.hostPlatform.isDarwin [
              fixDarwinDylibNames
            ] ++ commonArgs.nativeBuildInputs;
            
            cargoBuildExtraArgs = "-p capi";
            installPhaseCommand = ''
              mkdir -p "$out/include"
              install -Dm 755 "./target/release/capi/ayatsuri.h" "$out/include/ayatsuri.h"
              mkdir -p "$out/lib"
              install -Dm 755 "./target/release/capi/libayatsuri_capi.*" "$out/lib"
              install -Dm 755 "./target/release/capi/ayatsuri_capi.*" "$out/lib/"
              mkdir -p "$out/share/doc/libayatsuri"
              install -Dm 755 "./target/release/capi/LICENSE" "$out/share/doc/libayatsuri"
            '';
          }
        );
      in f {
        crates = { inherit ayatsuri libayatsuri; };
        inherit pkgs craneLib;
      }
    ));
  in {
    packages = forAllSystems ({crates, ...}: crates // {
      default = crates.ayatsuri;
    });

    apps = forAllSystems ({crates, ...}: {
      default = {
        type = "app";
        program = "${crates.ayatsuri}";
      };
    });
    
    devShells = forAllSystems ({craneLib, pkgs, crates, ...}: {
      default = craneLib.devShell {
        inputsFrom = [ crates.ayatsuri crates.libayatsuri ];
        packages = with pkgs; [
          rust-analyzer
        ];
      };
    });
  };
}
