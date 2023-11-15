FROM ubuntu:latest

# 在 alpine 中設置時區
ENV TZ=Asia/Taipei

# ubuntu 設定時區
RUN apt-get update && apt-get install -y tzdata ca-certificates
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone

# 設定工作目錄
WORKDIR /app

# 暴露連接埠
EXPOSE 9001

ADD ./target/x86_64-unknown-linux-gnu/release/stock_crawler .
ADD .env .
ADD ./app.json .
ADD ./etc/ssl ./etc/ssl

#RUN chmod +x /app/stock_crawler

VOLUME ["/app/log"]

# 運行應用程序
CMD ["/app/stock_crawler"]