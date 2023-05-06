#! /bin/bash

export built_path=./target/release
export app_path=./bin
export binary_name=rust_tutorial

function start() {
  # pid=$(pgrep $binary_name)
  pid=$(pidof $binary_name)
  # echo "$binary_name pid  = $pid"

  if [ "$pid" == "" ]; then
    cd $app_path || exit
    ./$binary_name > nohup.out 2>&1 &
    echo "$app_path/$binary_name start"
  else
    echo "$app_path/$binary_name is running already"
  fi
}

function stop() {
  if [ -f "$app_path/nohup.out" ]; then
    mv "$app_path/nohup.out" $app_path/log_backup/nohup.out."$(date "+%Y%m%d-%H%M%S")"
  fi

  pid=$(pidof $binary_name)

  if [ "$pid" != "" ]; then
    kill -SIGTERM "$pid"
    echo "$app_path/$binary_name  stop"
  else
    echo "pid of $app_path/$binary_name is empty"
  fi
}

function update() {
  build
  sleep 1
  stop
  sleep 1
  move
  sleep 1
  start
}

function restart() {
  stop
  sleep 1
  start
}

function build() {
  cargo update
  cargo build --release
  echo "success building"
}

function move() {
  if [ -f "$built_path/$binary_name" ]; then
    backup_name="$binary_name.$(date "+%Y%m%d-%H%M%S")"
    mv "$app_path/$binary_name" "$app_path/$backup_name"
    mv "$built_path/$binary_name" "$app_path/$binary_name"
    chmod -x "$app_path/$backup_name"
    chmod +x "$app_path/$binary_name"
    echo "moving the file from $built_path/$binary_name to $app_path/$binary_name is success"
  else
    echo "file $built_path/$binary_name does not exists."
  fi
}

function help() {
  echo "$0 start|stop|restart|update|move"
}

if [ "$1" == "start" ]; then
  start
elif [ "$1" == "stop" ]; then
  stop
elif [ "$1" == "restart" ]; then
  restart
elif [ "$1" == "update" ]; then
  update
elif [ "$1" == "move" ]; then
  move
else
  help
fi
