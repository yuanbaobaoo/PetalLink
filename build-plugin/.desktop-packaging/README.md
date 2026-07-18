# Internal desktop packaging backend

This directory is not a Kotlin Toolchain module and must not be imported into
IntelliJ IDEA as a Gradle project. It only preserves Compose Desktop DMG,
signing, and notarization support behind `./kotlin do packageDmg` and
`./kotlin do releaseDmg`.

For normal development, mark this directory as `Excluded` in the IDEA Project
view and use the repository-level Kotlin Toolchain model.
