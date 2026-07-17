// shared 模块：JVM + Compose Multiplatform Desktop
// 产出可执行 JAR / macOS .app，通过 Compose Desktop Window 运行

plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.kotlinSerialization)
    alias(libs.plugins.sqldelight)
    alias(libs.plugins.composeMultiplatform)
    alias(libs.plugins.composeCompiler)
}

val petalLinkVersion = providers.gradleProperty("petalLinkVersion").get()
// 构建档案（dev/release）是 bundle id、运行时数据目录与 LaunchAgent label 的唯一真相源。
// 用 -Prelease=true 切换：true → release（prod bundle id，沿用原 Tauri 老用户数据目录）；
// 不带 / false / 任意非 true 值 → dev（附加 -dev 后缀，数据目录/单实例锁/开机自启与 release 完全隔离）。
// 因此 run/jvmTest/本地 packageDmg 默认都落到 dev 数据目录，不污染正式数据；正式发布才 -Prelease=true。
val isRelease = providers.gradleProperty("release").map { it.equals("true", ignoreCase = true) }.orElse(false).get()
val buildProfile = if (isRelease) "release" else "dev"
val prodBundleId = "io.github.yuanbaobaoo.PetalLink"
val devBundleId = "$prodBundleId-dev"
val bundleId = if (isRelease) prodBundleId else devBundleId
val updateEndpoint = providers.environmentVariable("PETALLINK_UPDATE_ENDPOINT")
    .orElse("https://github.com/yuanbaobaoo/PetalLink/releases/latest/download/PetalLink-update.json")
val updateTeamId = providers.environmentVariable("PETALLINK_UPDATE_TEAM_ID").orElse("")
val generatedBuildInfo = layout.buildDirectory.dir("generated/petallink-build-info/kotlin")
val generatePetalLinkBuildInfo by tasks.registering {
    inputs.property("version", petalLinkVersion)
    inputs.property("buildProfile", buildProfile)
    inputs.property("bundleId", bundleId)
    inputs.property("updateEndpoint", updateEndpoint)
    inputs.property("updateTeamId", updateTeamId)
    outputs.dir(generatedBuildInfo)
    doLast {
        val output = generatedBuildInfo.get().file(
            "io/github/yuanbaobaoo/petallink/core/BuildInfo.kt",
        ).asFile
        output.parentFile.mkdirs()
        output.writeText(
            """package io.github.yuanbaobaoo.petallink.core

object BuildInfo {
    const val VERSION: String = "$petalLinkVersion"
    const val BUILD_PROFILE: String = "$buildProfile"
    const val BUNDLE_ID: String = "$bundleId"
    const val UPDATE_ENDPOINT: String = "${updateEndpoint.get()}"
    const val UPDATE_TEAM_ID: String = "${updateTeamId.get()}"
}
""",
        )
    }
}

kotlin {
    jvm()

    sourceSets {
        jvmMain { kotlin.srcDir(generatedBuildInfo) }
        commonMain {
            dependencies {
                implementation(libs.kotlin.coroutines)
                implementation(libs.kotlin.serialization.json)
                implementation(libs.ktor.core)
                implementation(libs.ktor.content.negotiation)
                implementation(libs.ktor.serialization.json)
                implementation(libs.sqldelight.coroutines)
            }
        }
        jvmMain {
            dependencies {
                implementation(compose.desktop.currentOs)
                implementation(libs.ktor.cio)
                implementation(libs.sqldelight.jvm)
                implementation(libs.sqlite.jdbc)
                implementation(libs.jna)
                implementation(libs.jna.platform)
            }
        }
        commonTest {
            dependencies {
                implementation(kotlin("test"))
                implementation(libs.kotlin.coroutines)
                implementation(libs.kotlin.coroutines.test)
                implementation(libs.ktor.mock)
            }
        }
    }
}

tasks.named("compileKotlinJvm").configure { dependsOn(generatePetalLinkBuildInfo) }

// SQLDelight 数据库配置
sqldelight {
    databases {
        create("PetalLinkDatabase") {
            packageName.set("io.github.yuanbaobaoo.petallink.data")
        }
    }
}

// Compose Desktop 可运行分发
compose.desktop {
    application {
        mainClass = "io.github.yuanbaobaoo.petallink.MainKt"
        nativeDistributions {
            // SQLite JDBC 通过反射使用 JDBC；jlink 的静态分析无法自动发现该模块。
            modules("java.sql")
            targetFormats(org.jetbrains.compose.desktop.application.dsl.TargetFormat.Dmg)
            packageName = "PetalLink"
            packageVersion = petalLinkVersion
            description = "华为云盘 macOS 客户端开源版"
            vendor = "PetalLink"
            macOS {
                bundleID = bundleId
                minimumSystemVersion = "12.0"
                packageVersion = petalLinkVersion
                packageBuildVersion = petalLinkVersion
                dmgPackageVersion = petalLinkVersion
                dmgPackageBuildVersion = petalLinkVersion
                iconFile.set(project.file("src/jvmMain/resources/icon.icns"))
                entitlementsFile.set(project.file("src/jvmMain/resources/Entitlements.plist"))
                runtimeEntitlementsFile.set(project.file("src/jvmMain/resources/RuntimeEntitlements.plist"))
                signing {
                    sign.set(providers.environmentVariable("PETALLINK_MAC_SIGN").map { it.toBoolean() }.orElse(false))
                    identity.set(providers.environmentVariable("PETALLINK_MAC_SIGN_IDENTITY"))
                    keychain.set(providers.environmentVariable("PETALLINK_MAC_SIGN_KEYCHAIN"))
                }
                notarization {
                    appleID.set(providers.environmentVariable("PETALLINK_NOTARY_APPLE_ID"))
                    password.set(providers.environmentVariable("PETALLINK_NOTARY_PASSWORD"))
                    teamID.set(providers.environmentVariable("PETALLINK_NOTARY_TEAM_ID"))
                }
            }
        }
    }
}
