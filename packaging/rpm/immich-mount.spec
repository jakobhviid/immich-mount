# Packages the prebuilt static binary (release.yml, "Package"); nothing is compiled here.
# rpmbuild -bb --target <arch> --define "pkg_version <version>" --define "payload <directory>" immich-mount.spec
%global debug_package %{nil}
%global __strip /bin/true
%global source_date_epoch_from_changelog 0

Name:           immich-mount
Version:        %{pkg_version}
Release:        1
Summary:        Your Immich photos and videos as a drive in your file manager
License:        MIT
URL:            https://github.com/jakobhviid/immich-mount
Requires:       fuse3
Requires:       hicolor-icon-theme
# The app's interface for the desktop installed, which `immich-mount app` (the launcher entry) opens.
Recommends:     (immich-mount-gnome if gnome-shell)
Recommends:     (immich-mount-kde if plasma-workspace)
# Its file trigger refreshes the MIME cache, which registers the app.immich scheme for the browser's redirect.
Recommends:     desktop-file-utils

%description
Also known as Immich drive. A FUSE daemon and command line that show a self-hosted
Immich library as an ordinary folder, downloading originals only when something
opens them, and uploading files copied into its Upload folder.

Immich Mount is an independent, unofficial project. It is not affiliated with,
endorsed by, or sponsored by the Immich project or FUTO. "Immich" is used only to
describe the server this software works with, and remains the mark of its owner.

%install
install -Dm755 %{payload}/immich-mount %{buildroot}%{_bindir}/immich-mount
install -Dm644 %{payload}/immich-mount.service %{buildroot}/usr/lib/systemd/user/immich-mount.service
# The icon both apps and the notifications use, here because both app packages depend on this one.
install -Dm644 %{payload}/dev.hviid.ImmichMount.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/dev.hviid.ImmichMount.svg
install -Dm644 %{payload}/dev.hviid.ImmichMount-symbolic.svg %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/dev.hviid.ImmichMount-symbolic.svg
# The app's one launcher entry, which opens the interface native to the desktop, and its AppStream data.
install -Dm644 %{payload}/dev.hviid.ImmichMount.desktop %{buildroot}%{_datadir}/applications/dev.hviid.ImmichMount.desktop
install -Dm644 %{payload}/dev.hviid.ImmichMount.metainfo.xml %{buildroot}%{_datadir}/metainfo/dev.hviid.ImmichMount.metainfo.xml
# The translations of both apps and the daemon's notifications.
mkdir -p %{buildroot}%{_datadir}/locale
cp -r %{payload}/locale/. %{buildroot}%{_datadir}/locale/
install -Dm644 %{payload}/LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 %{payload}/THIRD-PARTY-LICENSES.md %{buildroot}%{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md
install -Dm644 %{payload}/README.md %{buildroot}%{_docdir}/%{name}/README.md

%files
%{_bindir}/immich-mount
/usr/lib/systemd/user/immich-mount.service
%{_datadir}/icons/hicolor/scalable/apps/dev.hviid.ImmichMount.svg
%{_datadir}/icons/hicolor/symbolic/apps/dev.hviid.ImmichMount-symbolic.svg
%{_datadir}/applications/dev.hviid.ImmichMount.desktop
%{_datadir}/metainfo/dev.hviid.ImmichMount.metainfo.xml
%{_datadir}/locale/*/LC_MESSAGES/immich-mount.mo
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md
%doc %{_docdir}/%{name}/README.md
