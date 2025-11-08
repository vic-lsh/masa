# compose post (root of all services on a high level)
ghz --insecure \
  --proto ./proto/compose_post.proto \
  --call compose_post.ComposePostService.ComposePost \
  -d '{"req_id": 1, 
        "username": "tarangd",
        "user_id": 23,
        "text" : "Hello World",
        "media_ids": [1, 2],
        "media_types" : ["image", "text"],
        "post_type": "POST",
        "carrier": {}}' \
  localhost:8080


# ghz --insecure \
#   --proto ./proto/compose_post.proto \
#   --call compose_post.ComposePostService.ComposePost \
#   -d '{"req_id": 1, 
#         "username": "neeld",
#         "user_id": 28,
#         "text" : "Hello World to Tarang!",
#         "media_ids": [3, 4],
#         "media_types" : ["image", "text"],
#         "post_type": "POST",
#         "carrier": {}}' \
#   localhost:8080

# user timeline: write user timeline
ghz --insecure \
  --proto ./proto/user_timeline.proto \
  --call user_timeline.UserTimelineService.WriteUserTimeline \
  -d '{"req_id": 1, 
        "post_id": "1",
        "user_id": 23,
        "timestamp" : 251107210802,
        "carrier": {}}' \
  localhost:8090

# user timeline: read user timeline
ghz --insecure \
  --proto ./proto/user_timeline.proto \
  --call user_timeline.UserTimelineService.ReadUserTimeline \
  -d '{"req_id": 1, 
        "user_id": 23,
        "start" : 1234567,
        "stop" : 29,
        "carrier": {}}' \
  localhost:8090

# home timeline: write home timeline
ghz --insecure \
  --proto ./proto/home_timeline.proto \
  --call home_timeline.HomeTimelineService.WriteHomeTimeline \
  -d '{"req_id": 1, 
        "post_id": "1",
        "user_id": 23,
        "timestamp" : 251107210802,
        "user_mentions_id" : [24, 25],
        "carrier": {}}' \
  localhost:8081

# home timline: read hometimline
ghz --insecure \
  --proto ./proto/home_timeline.proto \
  --call home_timeline.HomeTimelineService.ReadHomeTimeline \
  -d '{"req_id": 1, 
        "user_id": 23,
        "start" : 1234567,
        "stop" : 25,
        "carrier": {}}' \
  localhost:8081

# user service: register user
ghz --insecure \
  --proto ./proto/user.proto \
  --call user.UserService.RegisterUser \
  -d '{"req_id": 1, 
        "first_name": "xyz",
        "last_name" : "abc",
        "username" : "xaybzc",
        "password" : "hello123",
        "carrier": {}}' \
  localhost:8088

