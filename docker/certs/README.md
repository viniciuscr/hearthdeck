# Corporate MITM-proxy root/intermediate CA certificates (PEM), if your
# network requires them.
#
# If your machine is behind a TLS-intercepting proxy (this repo hit Zscaler
# doing this for outbound HTTPS while building this image -- see
# ../README.md), a fresh container won't trust that proxy's certificate the
# way your host OS already does, and `pacman`/`curl`/`git`/`pip` inside the
# build will fail with certificate-verification errors. Export your
# organization's root (and any intermediate) CA as .pem here and the
# Dockerfile will install it into the container's trust store automatically.
#
# macOS export example (adjust the certificate name to match what's in your
# Keychain -- check Keychain Access.app, or
# `security find-certificate -a /Library/Keychains/System.keychain | grep labl`):
#
#   security find-certificate -c "Your Corporate Root CA" -p \
#     /Library/Keychains/System.keychain > docker/certs/corporate-root-ca.pem
#
# Everything in this directory except this README is gitignored -- these
# certs are specific to your network/employer, not to Hearthdeck, and should
# not be committed.
