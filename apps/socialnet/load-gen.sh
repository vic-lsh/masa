# Sending load to compose post (root of all services on a high level
ghz --insecure \
  --rps 100 \
  --duration 15s \
  --proto ./proto/compose_post.proto \
  --call compose_post.ComposePostService.ComposePost \
  --data-file ./payloads.json \
  localhost:8080


# Note: when running compose post, The ./payload.json file acts as a dynamic "blueprint" for 
# the load test, using the {{.RequestNumber}} variable to generate a unique user_id for every 
# single request sent by ghz.

# This is needed because otherwise, the same request will be send to the exact same user ID, 
# causing MongoDB to lock that single document and force all other requests to wait in a long 
# line (row locking).

# By randomizing the data via this file, the write operations get distributed across many 
# different documents simultaneously, which removes the artificial "hot key" bottleneck and 
# allows your system to demonstrate its true parallel performance.


# OTHER SERVICES THAT WE CAN SEND LOAD TO:

# ghz --insecure \
#   --proto ./proto/textservice.proto \
#   --call textservice.TextService.ComposeText \
#   -d '{"text": "trying some example text. Here is a link https://instagram.com/solacekitty213"}' \
#   localhost:8085



# ghz --insecure \
#   --proto ./proto/url_shorten.proto \
#   --call url_shorten.UrlShortenService.ComposeUrls \
#   -d '{
#     "req_id": 12345,
#     "urls": [
#       "https://www.instagram.com/solacekitty231",
#       "https://bit.ly/example"
#     ]
#   }' \
#   localhost:8087

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

# # user timeline: write user timeline
# ghz --insecure \
#   --proto ./proto/user_timeline.proto \
#   --call user_timeline.UserTimelineService.WriteUserTimeline \
#   -d '{"req_id": 1, 
#         "post_id": "1",
#         "user_id": 23,
#         "timestamp" : 1234,
#         "carrier": {}}' \
#   localhost:8090

# # user timeline: read user timeline
# ghz --insecure \
#   --proto ./proto/user_timeline.proto \
#   --call user_timeline.UserTimelineService.ReadUserTimeline \
#   -d '{"req_id": 1, 
#         "user_id": 23,
#         "start" : 1234567,
#         "stop" : 29,
#         "carrier": {}}' \
#   localhost:8090

# home timeline: write home timeline
# ghz --insecure \
#   --proto ./proto/home_timeline.proto \
#   --call home_timeline.HomeTimelineService.WriteHomeTimeline \
#   -d '{"req_id": 1, 
#         "post_id": "1",
#         "user_id": 23,
#         "timestamp" : 251107210802,
#         "user_mentions_id" : [24, 25],
#         "carrier": {}}' \
#   localhost:8081

# # home timline: read hometimline
# ghz --insecure \
#   --proto ./proto/home_timeline.proto \
#   --call home_timeline.HomeTimelineService.ReadHomeTimeline \
#   -d '{"req_id": 1, 
#         "user_id": 23,
#         "start" : 1234567,
#         "stop" : 25,
#         "carrier": {}}' \
#   localhost:8081

# user service: register user
# ghz --insecure \
#   --proto ./proto/user.proto \
#   --call user.UserService.RegisterUser \
#   -d '{"req_id": 1, 
#         "first_name": "apple",
#         "last_name" : "fruite",
#         "username" : "appleFruit",
#         "password" : "apples",
#         "carrier": {}}' \
#   localhost:8088

