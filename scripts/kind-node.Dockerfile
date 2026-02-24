ARG BASE_IMAGE=kindest/node:v1.32.0
FROM ${BASE_IMAGE}

RUN mv /usr/local/bin/entrypoint /usr/local/bin/entrypoint.real
COPY kind-node-entrypoint.sh /usr/local/bin/entrypoint
RUN chmod 0755 /usr/local/bin/entrypoint
