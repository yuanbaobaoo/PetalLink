// shared 模块：JVM + Compose Multiplatform Desktop
// 产出可执行 JAR / macOS .app，通过 Compose Desktop Window 运行

plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.kotlinSerialization)
    alias(libs.plugins.sqldelight)
    alias(libs.plugins.composeMultiplatform)
    alias(libs.plugins.composeCompiler)
}

kotlin {
    jvm()

    sourceSets {
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
            }
        }
    }
}

// SQLDelight 数据库配置
sqldelight {
    databases {
        create("PetalLinkDatabase") {
            packageName.set("io.github.yuanbaobaao.petallink.data")
        }
    }
}

// Compose Desktop 可运行分发
compose.desktop {
    application {
        mainClass = "io.github.yuanbaobaao.petallink.MainKt"
        nativeDistributions {
            targetFormats(org.jetbrains.compose.desktop.application.dsl.TargetFormat.Dmg, org.jetbrains.compose.desktop.application.dsl.TargetFormat.Msi)
            packageName = "PetalLink"
            packageVersion = "1.0.0"
            macOS { bundleID = "io.github.yuanbaobaao.petallink.macos" }
        }
    }
}
