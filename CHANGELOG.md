## [0.34.2](https://github.com/MSpiechowicz/pairpane/compare/v0.34.1...v0.34.2) (2026-11-25)


### Bug Fixes

* **release:** the release commit check accepts the bump of every workspace crate (#88) ([487c6c3](https://github.com/MSpiechowicz/pairpane/commit/487c6c3973738b429b93eaa6d65e63914bd705fe))

## [0.34.1](https://github.com/MSpiechowicz/pairpane/compare/v0.34.0...v0.34.1) (2026-11-25)


### Bug Fixes

* **release:** bump every workspace crate in Cargo.lock (#87) ([c152d4b](https://github.com/MSpiechowicz/pairpane/commit/c152d4b98c6bfa7b3a0bbd4b3d66adbbb1ef28ef))

# [0.34.0](https://github.com/MSpiechowicz/pairpane/compare/v0.33.7...v0.34.0) (2026-11-25)


### Features

* **link:** device identity, one-time code pairing and the encrypted connection (#86) ([eb06fe0](https://github.com/MSpiechowicz/pairpane/commit/eb06fe0342d5395b7dbcf91d06513913dd51feaf))

## [0.33.7](https://github.com/MSpiechowicz/pairpane/compare/v0.33.6...v0.33.7) (2026-11-25)


### Bug Fixes

* **ops:** a move across disks keeps a macOS ACL (#85) ([487fdf3](https://github.com/MSpiechowicz/pairpane/commit/487fdf355dfffe69aa7c5d9594701ebd5b424436)), closes [#80](https://github.com/MSpiechowicz/pairpane/issues/80)

## [0.33.6](https://github.com/MSpiechowicz/pairpane/compare/v0.33.5...v0.33.6) (2026-11-25)


### Bug Fixes

* **ops:** a no-replace rename the system refuses on Windows, and by a link where the flag is missing (#84) ([5219d55](https://github.com/MSpiechowicz/pairpane/commit/5219d559cc5c4ae2e8aca14acf87b22a0a4f8651)), closes [#77](https://github.com/MSpiechowicz/pairpane/issues/77)

## [0.33.5](https://github.com/MSpiechowicz/pairpane/compare/v0.33.4...v0.33.5) (2026-11-25)


### Bug Fixes

* **edit:** keep the old content beside a file written in place until the new one is on the disk (#83) ([da5752f](https://github.com/MSpiechowicz/pairpane/commit/da5752ffe44db497f3108c399998e5871ccca6af)), closes [#76](https://github.com/MSpiechowicz/pairpane/issues/76)

## [0.33.4](https://github.com/MSpiechowicz/pairpane/compare/v0.33.3...v0.33.4) (2026-11-25)


### Bug Fixes

* **sync:** a mirror with an archive side looks at the source again before removing (#81) ([b7ebf8a](https://github.com/MSpiechowicz/pairpane/commit/b7ebf8af5fa607771518111e7dbd320c56465c1e)), closes [#75](https://github.com/MSpiechowicz/pairpane/issues/75)
* **rename:** a renamed item moved aside keeps its old name in its temporary one (#82) ([57e27df](https://github.com/MSpiechowicz/pairpane/commit/57e27df6e8f2d099a28dc96da20a7aa33c44dd9d)), closes [#74](https://github.com/MSpiechowicz/pairpane/issues/74)

## [0.33.3](https://github.com/MSpiechowicz/pairpane/compare/v0.33.2...v0.33.3) (2026-11-25)


### Bug Fixes

* **ops:** a move across disks takes extended attributes and streams along (#79) ([7f55932](https://github.com/MSpiechowicz/pairpane/commit/7f559322363de7bde9915860c20279a89e1f0146)), closes [#73](https://github.com/MSpiechowicz/pairpane/issues/73)

## [0.33.2](https://github.com/MSpiechowicz/pairpane/compare/v0.33.1...v0.33.2) (2026-11-25)


### Bug Fixes

* **ops:** data-loss fixes from the audit of the local core (#78) ([c364c02](https://github.com/MSpiechowicz/pairpane/commit/c364c021f03c02cada973de0a67a966ac3e2eefa))

## [0.33.1](https://github.com/MSpiechowicz/pairpane/compare/v0.33.0...v0.33.1) (2026-11-25)


### Performance Improvements

* **listing:** order only the first screen before the first batch (#72) ([8ee5546](https://github.com/MSpiechowicz/pairpane/commit/8ee5546f67a8566be23ee64cd5ec5cdb75d5cab1))

# [0.33.0](https://github.com/MSpiechowicz/pairpane/compare/v0.32.0...v0.33.0) (2026-11-24)


### Features

* **view:** git status column (#66) ([d29236a](https://github.com/MSpiechowicz/pairpane/commit/d29236a0ee92d9f791433e13d009a5e05ee79106)), closes [#60](https://github.com/MSpiechowicz/pairpane/issues/60)

# [0.32.0](https://github.com/MSpiechowicz/pairpane/compare/v0.31.0...v0.32.0) (2026-11-23)


### Features

* **view:** dimensions and checksum columns (#65) ([6addaac](https://github.com/MSpiechowicz/pairpane/commit/6addaac946b81a2069f1b3b4e2d8022c13be1087))

# [0.31.0](https://github.com/MSpiechowicz/pairpane/compare/v0.30.0...v0.31.0) (2026-11-23)


### Features

* **view:** thumbnails view, made in the sandboxed helper (#63) ([3fa1420](https://github.com/MSpiechowicz/pairpane/commit/3fa1420a9aed5bcda9d4b95284bb5aeec0da502c)), closes [#59](https://github.com/MSpiechowicz/pairpane/issues/59)

### Bug Fixes

* **view:** make the thumbnail container's profile one thread at a time (#64) ([c42b72c](https://github.com/MSpiechowicz/pairpane/commit/c42b72c0986ef84d7f1cfeab5595aecaddfc742b))

# [0.30.0](https://github.com/MSpiechowicz/pairpane/compare/v0.29.0...v0.30.0) (2026-11-23)


### Features

* **ui:** brief view and per-tab layout (#62) ([179928b](https://github.com/MSpiechowicz/pairpane/commit/179928bf39916ca2170d176dcf61350cf2302676))

# [0.29.0](https://github.com/MSpiechowicz/pairpane/compare/v0.28.0...v0.29.0) (2026-11-23)


### Features

* **ui:** flat view (Ctrl+B) (#61) ([4b44c8f](https://github.com/MSpiechowicz/pairpane/commit/4b44c8fa5852d1bfc3a722243213ca3b5be4085a)), closes [#58](https://github.com/MSpiechowicz/pairpane/issues/58)

# [0.28.0](https://github.com/MSpiechowicz/pairpane/compare/v0.27.0...v0.28.0) (2026-11-23)


### Features

* **ui:** settings page (#57) ([78cb475](https://github.com/MSpiechowicz/pairpane/commit/78cb475666ddccef7203c8dd1de5633146a03e6a)), closes [#53](https://github.com/MSpiechowicz/pairpane/issues/53)

# [0.27.0](https://github.com/MSpiechowicz/pairpane/compare/v0.26.0...v0.27.0) (2026-11-23)


### Features

* **shell:** command line in the dock and Open terminal here (#56) ([efd403a](https://github.com/MSpiechowicz/pairpane/commit/efd403a027cd91adc843a8251aab8354c73eb1ae)), closes [#52](https://github.com/MSpiechowicz/pairpane/issues/52)

# [0.26.0](https://github.com/MSpiechowicz/pairpane/compare/v0.25.0...v0.26.0) (2026-11-23)


### Features

* **ui:** rebindable keymap with a Total Commander default (#55) ([30642d3](https://github.com/MSpiechowicz/pairpane/commit/30642d3802f5e1972ed8afe392662d453a597878)), closes [#51](https://github.com/MSpiechowicz/pairpane/issues/51)

# [0.25.0](https://github.com/MSpiechowicz/pairpane/compare/v0.24.0...v0.25.0) (2026-11-23)


### Features

* **ui:** Ctrl+K command palette (#54) ([17eb34b](https://github.com/MSpiechowicz/pairpane/commit/17eb34b5279df5ce5ab3258b5c29f083f98516c1)), closes [#50](https://github.com/MSpiechowicz/pairpane/issues/50)

# [0.24.0](https://github.com/MSpiechowicz/pairpane/compare/v0.23.0...v0.24.0) (2026-11-23)


### Features

* **diff:** compare files inside archives by content (#49) ([3571bc9](https://github.com/MSpiechowicz/pairpane/commit/3571bc9dca95677371ca1945ae097f214c2d2973)), closes [#43](https://github.com/MSpiechowicz/pairpane/issues/43)

# [0.23.0](https://github.com/MSpiechowicz/pairpane/compare/v0.22.0...v0.23.0) (2026-11-23)


### Features

* **sync:** undo a synchronize (#47) ([8ff77c0](https://github.com/MSpiechowicz/pairpane/commit/8ff77c09af5c721df4109c83c9770b25cde4c783)), closes [#41](https://github.com/MSpiechowicz/pairpane/issues/41)

# [0.22.0](https://github.com/MSpiechowicz/pairpane/compare/v0.21.0...v0.22.0) (2026-11-23)


### Features

* **sync:** synchronize into archives (#46) ([565f499](https://github.com/MSpiechowicz/pairpane/commit/565f4998c8aa5e24d87f17ed9b1943160606aacc)), closes [#40](https://github.com/MSpiechowicz/pairpane/issues/40)

# [0.21.0](https://github.com/MSpiechowicz/pairpane/compare/v0.20.0...v0.21.0) (2026-11-23)


### Features

* **sync:** mirror, removing what only the target has (#45) ([1b44945](https://github.com/MSpiechowicz/pairpane/commit/1b44945da60e2fb93023ed533928c312f0bfecdd)), closes [#39](https://github.com/MSpiechowicz/pairpane/issues/39)

# [0.20.0](https://github.com/MSpiechowicz/pairpane/compare/v0.19.0...v0.20.0) (2026-11-22)


### Features

* **diff:** compare files by content (#44) ([e3b6066](https://github.com/MSpiechowicz/pairpane/commit/e3b606693e25f0b580fea40d6f9f8fdeab46f1d9))

# [0.19.0](https://github.com/MSpiechowicz/pairpane/compare/v0.18.0...v0.19.0) (2026-11-22)


### Features

* **compare:** compare and synchronize folders (#42) ([6838bcb](https://github.com/MSpiechowicz/pairpane/commit/6838bcbc373143bda3bd7e49a5c4643b40e2b909))

# [0.18.0](https://github.com/MSpiechowicz/pairpane/compare/v0.17.0...v0.18.0) (2026-11-22)


### Features

* **rename:** multi-rename (Ctrl+M) with live preview, safe batch job, undo and presets (#38) ([3970245](https://github.com/MSpiechowicz/pairpane/commit/3970245648c96263c8ee01e3b922d9328dcc84ec))

# [0.17.0](https://github.com/MSpiechowicz/pairpane/compare/v0.16.0...v0.17.0) (2026-11-22)


### Features

* **find:** find files (Alt+F7) by name, content, size, date and attributes; duplicates, saved searches, results panel (#37) ([5f76987](https://github.com/MSpiechowicz/pairpane/commit/5f76987e7ba2e209a131d72e8b31ade17ec9f66c))

# [0.16.0](https://github.com/MSpiechowicz/pairpane/compare/v0.15.0...v0.16.0) (2026-11-22)


### Features

* archives — pack and add to archives (#36) ([c4a36b6](https://github.com/MSpiechowicz/pairpane/commit/c4a36b60bfde9149004c5ec7c60eb01ec66950ae))

# [0.15.0](https://github.com/MSpiechowicz/pairpane/compare/v0.14.0...v0.15.0) (2026-11-22)


### Features

* archives — browse as folders, view and extract (#35) ([f92e4bb](https://github.com/MSpiechowicz/pairpane/commit/f92e4bbd31c44e8a5cab164429938dc9ff92159f))

# [0.14.0](https://github.com/MSpiechowicz/pairpane/compare/v0.13.0...v0.14.0) (2026-11-21)


### Features

* F4 built-in editor and external editor ([97285ce](https://github.com/MSpiechowicz/pairpane/commit/97285ce47ee724d4aa32e94abe7b3a26096f188c)), closes [#15](https://github.com/MSpiechowicz/pairpane/issues/15)

# [0.13.0](https://github.com/MSpiechowicz/pairpane/compare/v0.12.0...v0.13.0) (2026-11-21)


### Features

* HEIC preview on Linux and Windows with the system's codecs ([4a18261](https://github.com/MSpiechowicz/pairpane/commit/4a18261a63afadbfaca425074ffcc04be604b859)), closes [#14](https://github.com/MSpiechowicz/pairpane/issues/14)

### Bug Fixes

* HEIC tests per platform and the WIC step that failed ([1aca2a3](https://github.com/MSpiechowicz/pairpane/commit/1aca2a32865a8284f9e14426a0ec2732d0f692f6)), closes [#14](https://github.com/MSpiechowicz/pairpane/issues/14)

# [0.12.0](https://github.com/MSpiechowicz/pairpane/compare/v0.11.0...v0.12.0) (2026-11-21)


### Features

* F3 lister and quick view ([78ce01a](https://github.com/MSpiechowicz/pairpane/commit/78ce01a9921cea2002811e389238c4f71d221e07)), closes [#14](https://github.com/MSpiechowicz/pairpane/issues/14)

# [0.11.0](https://github.com/MSpiechowicz/pairpane/compare/v0.10.0...v0.11.0) (2026-11-21)


### Features

* change hidden, system and archive attributes on Windows ([850d694](https://github.com/MSpiechowicz/pairpane/commit/850d694e8a748a6d92eda091ce07c690ac45b746)), closes [#13](https://github.com/MSpiechowicz/pairpane/issues/13)

# [0.10.0](https://github.com/MSpiechowicz/pairpane/compare/v0.9.0...v0.10.0) (2026-11-21)


### Features

* properties, permissions, timestamps, checksums and file split ([44489b4](https://github.com/MSpiechowicz/pairpane/commit/44489b4b9f29ce976029c55277d6f30392869b06)), closes [#13](https://github.com/MSpiechowicz/pairpane/issues/13)

# [0.9.0](https://github.com/MSpiechowicz/pairpane/compare/v0.8.0...v0.9.0) (2026-11-21)


### Features

* delete to Trash with undo, rename, new folder, new file and links ([e44efbb](https://github.com/MSpiechowicz/pairpane/commit/e44efbba60e0d01d67b4a0935be62af08266d8c4)), closes [#12](https://github.com/MSpiechowicz/pairpane/issues/12)

### Bug Fixes

* report taken names and missing items correctly on Windows ([1f64734](https://github.com/MSpiechowicz/pairpane/commit/1f647344e8498df21df6188ba52ddf147006a530))

# [0.8.0](https://github.com/MSpiechowicz/pairpane/compare/v0.7.0...v0.8.0) (2026-11-20)


### Features

* copy and move engine with a background queue ([2c9a8f1](https://github.com/MSpiechowicz/pairpane/commit/2c9a8f19d1aa4a6028d67cd4c913eee44d042084)), closes [#11](https://github.com/MSpiechowicz/pairpane/issues/11)

# [0.7.0](https://github.com/MSpiechowicz/pairpane/compare/v0.6.0...v0.7.0) (2026-11-20)


### Features

* startup splash (B1) ([a806218](https://github.com/MSpiechowicz/pairpane/commit/a806218e4f5c2fca15bd0a577518e66e6e9378df)), closes [#10](https://github.com/MSpiechowicz/pairpane/issues/10)

# [0.6.0](https://github.com/MSpiechowicz/pairpane/compare/v0.5.1...v0.6.0) (2026-11-20)


### Features

* pane tabs, drive menu, hotlist and history ([6284ee9](https://github.com/MSpiechowicz/pairpane/commit/6284ee96d442ac147366585bd89ddeda85c744a9)), closes [#9](https://github.com/MSpiechowicz/pairpane/issues/9)

## [0.5.1](https://github.com/MSpiechowicz/pairpane/compare/v0.5.0...v0.5.1) (2026-11-20)


### Bug Fixes

* keep panes and folder sizes off network shares and devices on Windows ([9149c3f](https://github.com/MSpiechowicz/pairpane/commit/9149c3fef7f07f83cd63ec28661f9834beb637cf))

# [0.5.0](https://github.com/MSpiechowicz/pairpane/compare/v0.4.0...v0.5.0) (2026-11-20)


### Features

* keyboard navigation, selection and quick filter in the panes ([9bffa6e](https://github.com/MSpiechowicz/pairpane/commit/9bffa6e219dd0baa45fb905203c1004350bd9d8c))

# [0.4.0](https://github.com/MSpiechowicz/pairpane/compare/v0.3.0...v0.4.0) (2026-11-20)


### Features

* fast folder listings in the panes ([8567de5](https://github.com/MSpiechowicz/pairpane/commit/8567de573442054027ab2a1759e1ff2d8f252fdc)), closes [#7](https://github.com/MSpiechowicz/pairpane/issues/7)

### Bug Fixes

* keep a pane's folder watch across reloads of the same folder ([0235adb](https://github.com/MSpiechowicz/pairpane/commit/0235adb7b2457f83f774f7a6ab2c0a9451443abb))

# [0.3.0](https://github.com/MSpiechowicz/pairpane/compare/v0.2.0...v0.3.0) (2026-11-20)


### Features

* app shell from the approved mockups ([2b24fc6](https://github.com/MSpiechowicz/pairpane/commit/2b24fc69c22f8450b196bba0ff032f5ef9e2d07a)), closes [#6](https://github.com/MSpiechowicz/pairpane/issues/6)

# [0.2.0](https://github.com/MSpiechowicz/pairpane/compare/v0.1.1...v0.2.0) (2026-11-20)


### Features

* install scripts and signed in-app updates ([dd41ade](https://github.com/MSpiechowicz/pairpane/commit/dd41adee7de0ae18da68f400ed7dfc3af8da253e)), closes [#4](https://github.com/MSpiechowicz/pairpane/issues/4)

## [0.1.1](https://github.com/MSpiechowicz/pairpane/compare/v0.1.0...v0.1.1) (2026-11-20)


### Bug Fixes

* **release:** hash the Bun download from stdin so Windows paths verify ([7794f06](https://github.com/MSpiechowicz/pairpane/commit/7794f06ba9106332b2d64afcf8db447b988300b4)), closes [#3](https://github.com/MSpiechowicz/pairpane/issues/3)

# [0.1.0](https://github.com/MSpiechowicz/pairpane/compare/v0.0.0...v0.1.0) (2026-11-20)


### Features

* scaffold the Tauri 2 + Svelte + Rust workspace ([13ccdb6](https://github.com/MSpiechowicz/pairpane/commit/13ccdb60804c135ba9bd5ad2c742522294891964)), closes [#1](https://github.com/MSpiechowicz/pairpane/issues/1)
