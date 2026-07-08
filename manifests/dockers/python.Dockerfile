FROM python:3.13-slim

ARG APP_NAME
ARG APP_MODULE

WORKDIR /app

COPY apps/${APP_NAME}/dist/*.whl .

RUN pip install --no-cache-dir *.whl && \
    rm -f *.whl

ENV PORT=8080
EXPOSE ${PORT}

LABEL \
    org.opencontainers.image.title="${APP_NAME}" \
    org.opencontainers.image.description="Python application from Nx monorepo" \
    security.non-root="true"

ENV APP_MODULE=${APP_MODULE}
CMD python -m ${APP_MODULE}
