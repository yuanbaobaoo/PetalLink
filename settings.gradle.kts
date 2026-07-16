// 根项目配置：JVM + Compose Multiplatform Desktop
pluginManagement {
    repositories {
        google()
        gradlePluginPortal()
        mavenCentral()
        maven { url = uri("https://maven.aliyun.com/repository/gradle-plugin") }
    }
}

dependencyResolutionManagement {
    // PREFER_SETTINGS：优先使用此声明的仓库，覆盖 init.gradle 注入的
    repositoriesMode.set(RepositoriesMode.PREFER_SETTINGS)
    repositories {
        google()
        mavenCentral()
        maven { url = uri("https://maven.aliyun.com/repository/public") }
    }
}

rootProject.name = "petal-link-cmp"
include(":shared")
