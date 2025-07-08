FROM ubuntu:22.04
RUN apt-get update -qq \
    && DEBIAN_FRONTEND=noninteractive apt-get -y dist-upgrade \
    && apt-get install -qy gnupg apt-transport-https ca-certificates curl \
    && echo 'deb [trusted=yes] https://kittengrid-repository-public.s3.eu-west-3.amazonaws.com/package-repository/ jammy main' > /etc/apt/sources.list.d/internal_repository.list \
    && apt-get update -qq
RUN DEBIAN_FRONTEND=noninteractive apt-get install -o Dpkg::Options::=--force-confdef -yq \
    --no-install-recommends consul ngrep jq dnsutils strace internal-certificate
COPY docker/consul/rootfs/ /
USER consul
ENTRYPOINT ["/entrypoint.sh"]

