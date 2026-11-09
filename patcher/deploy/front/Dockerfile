# Public front: Caddy with the website built in. Build context: the patcher/ directory.
#   docker build -f deploy/front/Dockerfile -t <registry>/patcher-front:<version> .
FROM caddy:2.11.9-alpine@sha256:0bbb055bda6793c999bde78b1b5ce3e89f829fb031af2cf50056ea1246559fee AS site
COPY website /website
# Only what a browser loads: no tests, package files, or dot-files.
RUN mkdir /site \
    && cd /website \
    && cp ./*.html ./*.css ./*.js /site/ \
    && cp -R assets data /site/ \
    && find /site -name '.*' -exec rm -rf {} + \
    && chmod -R a+rX,go-w /site

FROM caddy:2.11.9-alpine@sha256:0bbb055bda6793c999bde78b1b5ce3e89f829fb031af2cf50056ea1246559fee
COPY deploy/front/Caddyfile /etc/caddy/Caddyfile
COPY --from=site /site /srv/site
RUN mkdir -p /data /config /srv/repository && chown 1000:1000 /data /config
ENV XDG_DATA_HOME=/data XDG_CONFIG_HOME=/config
USER 1000:1000
EXPOSE 80 443 443/udp
CMD ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]
