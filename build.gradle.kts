import org.gradle.api.tasks.testing.Test
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// PetalLink：JVM + Compose Multiplatform Desktop（单模块，无 shared 外壳）
// 产出可执行 JAR / macOS .app，通过 Compose Desktop Window 运行

plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.kotlinSerialization)
    alias(libs.plugins.ksp)
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
val generatePetalLinkBuildInfo = tasks.register("generatePetalLinkBuildInfo") {
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
    jvmToolchain(25)
    jvm {
        compilerOptions {
            jvmTarget.set(JvmTarget.fromTarget("25"))
        }
    }

    sourceSets {
        jvmMain { kotlin.srcDir(generatedBuildInfo) }
        commonMain {
            dependencies {
                implementation(libs.kotlin.coroutines)
                implementation(libs.kotlin.serialization.json)
                implementation(libs.ktor.core)
                implementation(libs.ktor.content.negotiation)
                implementation(libs.ktor.serialization.json)
                implementation(libs.room.runtime)
                implementation(libs.sqlite.bundled)
            }
        }
        jvmMain {
            dependencies {
                implementation(compose.desktop.currentOs)
                implementation(libs.ktor.cio)
                implementation(libs.jna)
                implementation(libs.jna.platform)
                runtimeOnly(libs.slf4j.nop)
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

dependencies {
    add("kspJvm", libs.room.compiler)
}

tasks.named("compileKotlinJvm").configure { dependsOn(generatePetalLinkBuildInfo) }
tasks.matching { it.name.startsWith("ksp") }.configureEach { dependsOn(generatePetalLinkBuildInfo) }
tasks.withType<Test>().configureEach {
    // Room bundled SQLite 在测试 JVM 中通过 JNI 加载本地库，JDK 25 要求显式开放原生访问。
    jvmArgs("--enable-native-access=ALL-UNNAMED")
}

ksp {
    arg("room.schemaLocation", "$projectDir/schemas")
}

// Compose Desktop 可运行分发
compose.desktop {
    application {
        mainClass = "io.github.yuanbaobaoo.petallink.MainKt"
        // Skiko 通过 JNI 加载平台渲染库。JDK 25 下运行任务与打包后的启动器都必须显式开放原生访问。
        jvmArgs("--enable-native-access=ALL-UNNAMED")
        nativeDistributions {
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
