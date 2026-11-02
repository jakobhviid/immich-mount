# The KDE app, from a binary built in the same Fedora release (release.yml, "fedora-apps"); nothing is compiled here.
# rpmbuild -bb --define "pkg_version <version>" --define "payload <directory>" immich-mount-kde.spec
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           immich-mount-kde
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        Immich Mount for KDE Plasma: sign in, status and settings for the Immich drive in Dolphin
License:        MIT
URL:            https://github.com/jakobhviid/immich-mount
Requires:       immich-mount = %{version}
# QML modules the pages import; the Qt libraries themselves are found from the binary.
Requires:       kf6-kirigami
Requires:       kf6-qqc2-desktop-style
# xdg-open, through which Qt opens the browser for single sign-on and the folder in Dolphin.
Requires:       xdg-utils
# Its file trigger refreshes the MIME cache, which registers the app.immich scheme for the browser's redirect.
Requires:       desktop-file-utils

%description
Signs in to a self-hosted Immich server (single sign-on in the browser, email and
password, or an API key), keeps the credential in KWallet or another keyring, and
starts the mount that shows the library in Dolphin.

Immich Mount is an independent, unofficial project. It is not affiliated with,
endorsed by, or sponsored by the Immich project or FUTO. "Immich" is used only to
describe the server this software works with, and remains the mark of its owner.

%install
install -Dm755 %{payload}/immich-mount-kde %{buildroot}%{_bindir}/immich-mount-kde
install -Dm644 %{payload}/dev.hviid.ImmichMount.Kde.desktop %{buildroot}%{_datadir}/applications/dev.hviid.ImmichMount.Kde.desktop
install -Dm644 %{payload}/dev.hviid.ImmichMount.Kde.metainfo.xml %{buildroot}%{_metainfodir}/dev.hviid.ImmichMount.Kde.metainfo.xml
install -Dm644 %{payload}/LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 %{payload}/THIRD-PARTY-LICENSES.md %{buildroot}%{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md

%files
%{_bindir}/immich-mount-kde
%{_datadir}/applications/dev.hviid.ImmichMount.Kde.desktop
%{_metainfodir}/dev.hviid.ImmichMount.Kde.metainfo.xml
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md
