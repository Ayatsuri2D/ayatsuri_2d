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
            cargoBuildExtraArgs = "-p ayatsuri -p ayatsuri-launcher";
            installPhaseCommand = ''
              install -Dm755 "./target/release/ayatsuri" "$out/usr/bin/ayatsuri"
              install -Dm755 "./target/release/ayatsuri-launcher" "$out/usr/bin/ayatsuri-launcher"
              strip "$out/usr/bin/ayatsuri" "$out/usr/bin/ayatsuri-launcher" 2>/dev/null || true
              install -Dm644 "${here}/${appID}.desktop" "$out/usr/share/applications/${appID}.desktop"
              install -Dm644 "${here}/${appID}.mime.xml" "$out/usr/share/mime/packages/${appID}.xml"
              mkdir -p "$out/usr/share/metainfo"
              sed -e "s/@VERSION@/${version}/g" -e "s/@DATE@/$(date -u +%Y-%m-%d)/g" \
                "${here}/${appID}.metainfo.xml.in" > "$out/usr/share/metainfo/${appID}.metainfo.xml"
              mkdir -p "$out/usr/share/icons"
              cp -R "./assets/app-icon/hicolor" "$out/usr/share/icons/"

              mkdir -p "$out/usr/share/doc/ayatsuri2d"
              install -m644 "./LICENSE" "$out/usr/share/doc/ayatsuri2d/LICENSE"
              install -m644 "./LICENSING.md" "$out/usr/share/doc/ayatsuri2d/LICENSING.md"
              install -m644 "./README.md" "$out/usr/share/doc/ayatsuri2d/README.md"
              install -m644 "./assets/credits.tsv" "$out/usr/share/doc/ayatsuri2d/credits.tsv"
            '';
          }
        );
      in f { inherit ayatsuri; }
    ));
  in {
    packages = forAllSystems ({ayatsuri, ...}: {
      default = ayatsuri;
    });

    apps = forAllSystems ({ayatsuri, ...}: {
      default = {
        type = "app";
        program = "${ayatsuri}";
      };
    });
    
    devShells = forAllSystems ({...}: {
      default = [];
    });
  };
}
