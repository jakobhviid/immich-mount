# The GNOME app, from a binary built in the same Fedora release (release.yml, "gnome"); nothing is compiled here.
# rpmbuild -bb --define "pkg_version <version>" --define "payload <directory>" immich-mount-gnome.spec
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           immich-mount-gnome
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        Immich Mount's GNOME interface: sign in, status and settings for the Immich drive in Files
License:        MIT
URL:            https://github.com/jakobhviid/immich-mount
# The daemon, and the launcher entry that opens this interface outside Plasma.
Requires:       immich-mount = %{version}

%description
Signs in to a self-hosted Immich server (single sign-on in the browser, email and
password, or an API key), keeps the credential in the keyring, and starts the mount
that shows the library in Files.

Immich Mount is an independent, unofficial project. It is not affiliated with,
endorsed by, or sponsored by the Immich project or FUTO. "Immich" is used only to
describe the server this software works with, and remains the mark of its owner.

%install
install -Dm755 %{payload}/immich-mount-gnome %{buildroot}%{_bindir}/immich-mount-gnome
install -Dm644 %{payload}/LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 %{payload}/THIRD-PARTY-LICENSES.md %{buildroot}%{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md

%files
%{_bindir}/immich-mount-gnome
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/THIRD-PARTY-LICENSES.md
