FROM rust:bookworm as builder

WORKDIR /app

COPY . .

#RUN ls -la /app  && tail -f /dev/null
#RUN ls -la /app/etc/proto && tail -f /dev/null
RUN apt update && apt install -y protobuf-compiler openssl pkg-config libssl-dev libprotobuf-dev && rm -rf /var/lib/apt/lists/*

ENV PROTO_PATH="./etc/proto"


RUN cargo build --release

FROM debian:bookworm

# 設定環境變量，將時區設為 UTC+8
ENV TZ=Asia/Taipei

# 安裝 tzdata，並根據 TZ 環境變量自動設定時區
RUN apt-get update && apt-get install -y tzdata ca-certificates && \
    ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && \
    echo $TZ > /etc/timezone && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 從建構階段複製二進制檔案
COPY --from=builder /app/target/release/stock_crawler .

VOLUME ["/app/log"]

ADD .env .
ADD ./app.json .
ADD ./etc/ssl ./etc/ssl

# 設定容器啟動時執行您的應用
CMD ["/app/stock_crawler"]


