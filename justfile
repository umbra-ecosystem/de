deploy VERSION:
    git tag {{VERSION}}
    git push origin {{VERSION}}

# Dev bundle: Deus.app (debug) via cargo-bundle. macOS takes the menubar, Dock
# and About names from the bundle, never from the in-code menu title. The tool
# itself stays `de`.
app:
    cd crates/cli && cargo bundle
    if command -v codesign >/dev/null; then codesign --force --sign - target/debug/bundle/osx/Deus.app; fi
    @echo "open target/debug/bundle/osx/Deus.app"

# Publishing bundle: Deus.app (release) via cargo-bundle, ad-hoc signed.
bundle:
    cd crates/cli && cargo bundle --release
    if command -v codesign >/dev/null; then codesign --force --sign - target/release/bundle/osx/Deus.app; fi
    @echo "target/release/bundle/osx/Deus.app"
