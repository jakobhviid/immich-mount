# The Dolphin plugin: Open in Immich and Copy Immich Link (ROADMAP.md, "Open in Immich"). Built apart from the
# Rust workspace, against the KF6 of the distribution it ships in, and installed in Qt's plugin folder,
# kf6/kfileitemaction/, where Dolphin, Konqueror and Plasma's folder view look for file item actions:
#
#   cmake -S crates/immich-mount-kde/dolphin -B target/dolphin -DCMAKE_BUILD_TYPE=Release
#   cmake --build target/dolphin      # target/dolphin/bin/kf6/kfileitemaction/immich-mount-dolphin.so
cmake_minimum_required(VERSION 3.22)
project(immich-mount-dolphin LANGUAGES CXX)

find_package(ECM 6.0 REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)

find_package(Qt6 6.6 REQUIRED COMPONENTS Core DBus Gui Widgets)
find_package(KF6 6.0 REQUIRED COMPONENTS CoreAddons I18n KIO)

kcoreaddons_add_plugin(immich-mount-dolphin SOURCES immich-mount-dolphin.cpp INSTALL_NAMESPACE "kf6/kfileitemaction")
target_link_libraries(immich-mount-dolphin PRIVATE Qt6::DBus Qt6::Gui Qt6::Widgets KF6::CoreAddons KF6::I18n
                      KF6::KIOGui KF6::KIOWidgets)
# Only the libraries it calls, so a package depends on no more of KF6 than that.
target_link_options(immich-mount-dolphin PRIVATE "LINKER:--as-needed")
