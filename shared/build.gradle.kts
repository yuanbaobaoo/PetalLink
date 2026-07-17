// shared 模块：JVM + Compose Multiplatform Desktop
// 产出可执行 JAR / macOS .app，通过 Compose Desktop Window 运行

import java.nio.file.Files
import java.nio.file.Path

plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.kotlinSerialization)
    alias(libs.plugins.sqldelight)
    alias(libs.plugins.composeMultiplatform)
    alias(libs.plugins.composeCompiler)
}

val petalLinkVersion = providers.gradleProperty("petalLinkVersion").get()
// 构建档案（dev/release）是 bundle id、运行时数据目录与 LaunchAgent label 的唯一真相源。
// 默认 release：沿用原 Tauri 的 prod bundle id，保证老用户数据目录与单实例锁不变。
// dev：附加 -dev 后缀，与 release 包在系统层（LaunchServices）、数据目录、开机自启上完全隔离。
val buildProfile = providers.gradleProperty("petalLinkBuildProfile").orElse("release").get().lowercase()
require(buildProfile == "dev" || buildProfile == "release") {
    "petalLinkBuildProfile 仅支持 dev/release，当前值：$buildProfile"
}
val prodBundleId = "io.github.yuanbaobaoo.PetalLink"
val devBundleId = "$prodBundleId-dev"
val bundleId = if (buildProfile == "dev") devBundleId else prodBundleId
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

/**
 * 修复 jpackage --type dmg 的 entitlements 丢失问题。
 *
 * jpackage 从 --app-image 封 DMG 时，会用内部复制/重签流程，adhoc 签名场景下会丢掉主可执行的
 * entitlements（createDistributable 阶段产出的 app-image 本身是带 entitlements 的）。
 * 该任务在 packageDmg 之后，用 ditto + hdiutil 从带 entitlements 的 app-image 重新封一个 DMG，
 * 覆盖 jpackage 产物。ditto 保留全部签名元数据，hdiutil UDZO 压缩，entitlements 完整保留。
 */
val appImageDir = layout.buildDirectory.dir("compose/binaries/main/app/PetalLink.app")
val dmgOutputDir = layout.buildDirectory.dir("compose/binaries/main/dmg")
val repackDmgForEntitlements by tasks.registering {
    description = "从 app-image 用 hdiutil 重封 DMG，修复 jpackage 丢失 entitlements 的问题。"
    group = "compose desktop packaging"
    dependsOn("packageDmg")
    inputs.dir(appImageDir)
    outputs.file(dmgOutputDir.map { it.file("PetalLink-$petalLinkVersion.dmg") })
    doLast {
        val app = appImageDir.get().asFile
        check(app.isDirectory) { "app-image 不存在：$app（先执行 createDistributable）" }
        val dest = dmgOutputDir.get().asFile.apply { mkdirs() }
        val dmg = dest.resolve("PetalLink-$petalLinkVersion.dmg")
        val staging = dest.resolveSibling("tmp/dmg-repack-${System.currentTimeMillis()}")
        try {
            staging.mkdirs()
            // ditto 保留所有扩展属性与签名元数据（entitlements 随签名存储）。
            val ditto = ProcessBuilder("ditto", app.absolutePath, staging.resolve("PetalLink.app").absolutePath)
                .redirectErrorStream(true).start().also { it.waitFor() }
            check(ditto.exitValue() == 0) { "ditto 失败：${ditto.inputStream.bufferedReader().readText()}" }
            // Applications 软链，与 Compose DMG 布局一致。
            val appsLink = staging.resolve("Applications").toPath()
            if (!Files.exists(appsLink)) {
                Files.createSymbolicLink(appsLink, Path.of("/Applications"))
            }
            if (dmg.exists()) dmg.delete()
            val hdiutil = ProcessBuilder(
                "hdiutil", "create",
                "-volname", "PetalLink",
                "-srcfolder", staging.absolutePath,
                "-fs", "HFS+",
                "-format", "UDZO",
                "-imagekey", "zlib-level=9",
                dmg.absolutePath,
            ).redirectErrorStream(true).start().also { it.waitFor() }
            check(hdiutil.exitValue() == 0) { "hdiutil 失败：${hdiutil.inputStream.bufferedReader().readText()}" }
            logger.lifecycle("DMG 重封完成（保留 entitlements）：$dmg")
        } finally {
            staging.deleteRecursively()
        }
    }
}

// 让标准的 packageDmg 链路最终落到修复后的 DMG。
// packageDmg 由 Compose 插件延迟注册，用 matching/configureEach 在任务实际创建时再绑定。
tasks.matching { it.name == "packageDmg" }.configureEach { finalizedBy(repackDmgForEntitlements) }
