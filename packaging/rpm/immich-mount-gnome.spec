# The GNOME app, from a binary built in the same Fedora release (release.yml, "gnome"); nothing is compiled here.
# rpmbuild -bb --define "pkg_version <version>" --define "payload <directory>" immich-mount-gnome.spec
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           immich-mount-gnome
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        Immich Mount for GNOME: sign in, status and settings for the Immich drive in Files
License:        MIT
URL:            https://github.com/jakobhviid/immich-mount
Requires:       immich-mount = %{version}
# Its file trigger refreshes the MIME cache, which registers the app.immich scheme for the browser's redirect.
Requires:       desktop-file-utils

%description
Signs in to a self-hosted Immich server (single sign-on in the browser, email and
password, or an API key), keeps the credential in the keyring, and starts the mount
that shows the library in Files.

Immich Mount is an independent, unofficial project. It is not affiliated with,
endorsed by, or sponsored by the Immich project or FUTO. "Immich" is used only to
describe the server this software works with, and remains the mark of its owner.

%install
install -Dm755 %{payload}/immich-mount-gnome %{buildroot}%{_bindir}/immich-mount-gnome
install -Dm644 %{payload}/dev.hviid.ImmichMount.desktop %{buildroot}%{_datadir}/applications/dev.hviid.ImmichMount.desktop
install -Dm644 %{payload}/dev.hviid.ImmichMount.metainfo.xml %{buildroot}%{_metainfodir}/dev.hviid.ImmichMount.metainfo.xml
install -Dm644 %{payload}/LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 %{payload}/THIRD-PARTY-LICENSES.md %{buildroot}%{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md

%files
%{_bindir}/immich-mount-gnome
%{_datadir}/applications/dev.hviid.ImmichMount.desktop
%{_metainfodir}/dev.hviid.ImmichMount.metainfo.xml
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md
